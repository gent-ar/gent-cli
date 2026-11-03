use super::{
    Action, Arc, AtomicBool, BTreeMap, BufRead, BufReader, Command, CommandWrap, Duration, Error,
    Input, Instant, MAX_MESSAGE, Mutex, Ordering, Outbound, OwnedChild, Read, Spec, Stdio, Ticket,
    Value, Write, mpsc, thread,
};

type Reply = mpsc::SyncSender<Result<Value, Error>>;
type Pending = BTreeMap<String, (Reply, Instant, bool)>;
#[cfg(target_os = "linux")]
type Authority = Arc<crate::provider_egress::mcp_linux_child::RetainedAuthority>;

pub(super) struct Shared {
    pub(super) stopped: Arc<AtomicBool>,
    pub(super) settled: Arc<AtomicBool>,
    pub(super) retained: Arc<Mutex<Vec<Ticket>>>,
}

struct Pumps {
    reader: thread::JoinHandle<()>,
    stdin_pump: thread::JoinHandle<()>,
    reader_done: Arc<AtomicBool>,
    writer_done: Arc<AtomicBool>,
    write: mpsc::SyncSender<Outbound>,
    read: mpsc::Receiver<Input>,
}

fn write_retaining_operation(
    input: &mut impl Write,
    bytes: &[u8],
    operation: Option<Ticket>,
    held: &Mutex<Vec<Ticket>>,
) -> std::io::Result<()> {
    if let Some(ticket) = operation {
        held.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(ticket);
    }
    input.write_all(bytes).and_then(|()| input.flush())
}

#[cfg(not(target_os = "linux"))]
fn command(spec: &Spec) -> CommandWrap {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .current_dir(&spec.cwd)
        .env_clear()
        .envs(&spec.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    if let Some(value) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", value);
    }
    let mut command = CommandWrap::from(command);
    #[cfg(windows)]
    {
        command.wrap(process_wrap::std::CreationFlags(
            windows::Win32::System::Threading::CREATE_NO_WINDOW,
        ));
        command.wrap(process_wrap::std::JobObject);
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        command.wrap(process_wrap::std::ProcessGroup::leader());
    }
    command
}

pub(super) fn actor(
    mut spec: Spec,
    commands: mpsc::Receiver<Action>,
    ready: mpsc::SyncSender<Result<(), Error>>,
    shared: Shared,
    #[cfg(target_os = "linux")] native: Option<gent_linux_sandbox::host_stdio::Config>,
) -> Result<(), Error> {
    let mut startup = spec.startup.take();
    #[cfg(target_os = "linux")]
    let spec = Arc::new(spec);
    #[cfg(target_os = "linux")]
    let native_authority: Authority = Arc::new(
        crate::provider_egress::mcp_linux_child::RetainedAuthority::new(
            spec.clone(),
            startup.clone(),
        ),
    );
    #[cfg(not(target_os = "linux"))]
    let mut command = command(&spec);
    if shared.stopped.load(Ordering::Acquire) || !(spec.launch_guard)() {
        shared.settled.store(true, Ordering::Release);
        let _ = ready.send(Err(Error::Revoked));
        return Err(Error::Revoked);
    }
    #[cfg(target_os = "linux")]
    let spawned = native.ok_or(Error::Unprovable).and_then(|config| {
        crate::provider_egress::mcp_linux_child::spawn(config, native_authority.clone())
    });
    #[cfg(not(target_os = "linux"))]
    let spawned = command.spawn().map_err(|_| Error::Unprovable);
    let Ok(child) = spawned else {
        let _ = ready.send(Err(Error::Unprovable));
        return Err(Error::Unprovable);
    };
    let mut child = OwnedChild {
        child,
        settled: false,
    };
    #[cfg(target_os = "linux")]
    if shared.stopped.load(Ordering::Acquire) || !(spec.launch_guard)() {
        let _ = child.start_kill();
        let _ = child.wait();
        let _ = ready.send(Err(Error::Revoked));
        return Err(Error::Revoked);
    }
    let pumps = pumps(
        &mut child,
        #[cfg(target_os = "linux")]
        &spec,
        &shared,
    )?;
    let _ = ready.send(Ok(()));
    let mut pending = Pending::new();
    let _ = exchange(
        &spec,
        &commands,
        &shared.stopped,
        &pumps,
        &mut pending,
        &mut startup,
        #[cfg(target_os = "linux")]
        &native_authority,
    );
    shared.stopped.store(true, Ordering::Release);
    for (_, (reply, _, _)) in pending {
        let _ = reply.send(Err(Error::Unprovable));
    }
    child.settle();
    let Pumps {
        reader,
        stdin_pump,
        reader_done,
        writer_done,
        write,
        read,
    } = pumps;
    drop(write);
    drop(read);
    while !reader_done.load(Ordering::Acquire) || !writer_done.load(Ordering::Acquire) {
        thread::sleep(Duration::from_millis(10));
    }
    let _ = reader.join();
    let _ = stdin_pump.join();
    let mut held = shared
        .retained
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    shared.settled.store(true, Ordering::Release);
    held.clear();
    drop(held);
    drop(startup);
    drop(spec);
    Ok(())
}

fn pumps(
    child: &mut OwnedChild,
    #[cfg(target_os = "linux")] spec: &Spec,
    shared: &Shared,
) -> Result<Pumps, Error> {
    let input = child.stdin().take().ok_or(Error::Unprovable)?;
    let output = child.stdout().take().ok_or(Error::Unprovable)?;
    let (write, writes) = mpsc::sync_channel::<Outbound>(8);
    let (send, read) = mpsc::sync_channel::<Input>(16);
    let reader_done = Arc::new(AtomicBool::new(false));
    let writer_done = Arc::new(AtomicBool::new(false));
    let rd = reader_done.clone();
    let wr = writer_done.clone();
    let failed = send.clone();
    let reader = thread::spawn(move || {
        let mut reader = BufReader::new(output);
        loop {
            let mut bytes = Vec::new();
            let count =
                Read::take(&mut reader, MAX_MESSAGE as u64 + 1).read_until(b'\n', &mut bytes);
            if !matches!(count,Ok(n) if n>0) || bytes.len() > MAX_MESSAGE || !bytes.ends_with(b"\n")
            {
                break;
            }
            let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
                break;
            };
            if send.send(Input::Message(value)).is_err() {
                break;
            }
        }
        let _ = send.send(Input::Closed);
        rd.store(true, Ordering::Release);
    });
    let writer_stop = shared.stopped.clone();
    #[cfg(target_os = "linux")]
    let writer_launch_guard = spec.launch_guard.clone();
    let writer_retained = shared.retained.clone();
    let stdin_pump = thread::spawn(move || {
        let mut input = input;
        while let Ok(frame) = writes.recv() {
            #[cfg(target_os = "linux")]
            if !writer_launch_guard() {
                let _ = failed.send(Input::Closed);
                break;
            }
            if writer_stop.load(Ordering::Acquire)
                || frame.current.as_ref().is_some_and(|guard| !guard())
                || write_retaining_operation(
                    &mut input,
                    &frame.bytes,
                    frame.operation,
                    &writer_retained,
                )
                .is_err()
            {
                let _ = failed.send(Input::Closed);
                break;
            }
        }
        wr.store(true, Ordering::Release);
    });
    Ok(Pumps {
        reader,
        stdin_pump,
        reader_done,
        writer_done,
        write,
        read,
    })
}

fn exchange(
    spec: &Spec,
    commands: &mpsc::Receiver<Action>,
    stopped: &AtomicBool,
    pumps: &Pumps,
    pending: &mut Pending,
    startup: &mut Option<Ticket>,
    #[cfg(target_os = "linux")] native_authority: &Authority,
) -> Result<(), Error> {
    while !stopped.load(Ordering::Acquire) {
        #[cfg(target_os = "linux")]
        if !(spec.launch_guard)() {
            break;
        }
        while let Ok(input) = pumps.read.try_recv() {
            let Input::Message(value) = input else {
                return Ok(());
            };
            if super::super::mcp_secrets::contains(&value, &spec.secrets)
                || value.get("method").is_some()
            {
                return Ok(());
            }
            let id = serde_json::to_string(&value["id"]).map_err(|_| Error::Protocol)?;
            let Some((reply, _, initializing)) = pending.remove(&id) else {
                return Ok(());
            };
            if initializing && value.get("result").is_some() {
                startup.take();
                #[cfg(target_os = "linux")]
                native_authority.finish_startup();
            }
            let _ = reply.send(Ok(value));
        }
        if pending
            .values()
            .any(|(_, deadline, _)| Instant::now() >= *deadline)
        {
            break;
        }
        match commands.recv_timeout(Duration::from_millis(10)) {
            Ok(action) => {
                if !dispatch(action, &pumps.write, pending, spec.timeout)? {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => (),
        }
    }
    Ok(())
}

fn dispatch(
    action: Action,
    write: &mpsc::SyncSender<Outbound>,
    pending: &mut Pending,
    timeout: Duration,
) -> Result<bool, Error> {
    let (value, reply, guard, operation) = match action {
        Action::Request(value, reply, guard) => (value, Some(reply), guard, None),
        Action::Notify(value, guard, ticket) => (value, None, guard, ticket),
    };
    let mut bytes = serde_json::to_vec(&value).map_err(|_| Error::Protocol)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_MESSAGE
        || write
            .try_send(Outbound {
                bytes,
                current: Some(guard),
                operation,
            })
            .is_err()
    {
        return Ok(false);
    }
    let Some(reply) = reply else {
        return Ok(true);
    };
    let id = serde_json::to_string(&value["id"]).map_err(|_| Error::Protocol)?;
    let initializing = value["method"] == "tools/list";
    Ok(pending.len() < 8
        && pending
            .insert(id, (reply, Instant::now() + timeout, initializing))
            .is_none())
}

#[cfg(test)]
#[path = "provider_mcp_stdio_actor_tests.rs"]
mod operation_write_tests;
