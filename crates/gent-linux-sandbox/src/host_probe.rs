//! Trusted host credential probe ownership. This is not provider containment evidence.
//! The immutable helper waits on private stdin until it belongs to the new cgroup.
use gent_linux_cgroup_owner::{CgroupScope, DelegatedParent};
use gent_types::{RunVersionLock, SandboxResourceLimits};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Purpose {
    ClaudeCredentialProbe,
    GentMcpStdio,
    NodeMcpStdio,
}
fn allowed(
    purpose: Purpose,
    lock: &RunVersionLock,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> bool {
    match purpose {
        Purpose::ClaudeCredentialProbe => lock.provider == "claude" && valid_env(env),
        Purpose::NodeMcpStdio => {
            lock.provider == "node-mcp"
                && crate::node_entry::valid(Path::new(&lock.canonical_path), args, env)
        }
        Purpose::GentMcpStdio => {
            lock.provider == "gent-mcp"
                && crate::host_stdio::valid_shape(Path::new(&lock.canonical_path), args, env)
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    schema: u8,
    purpose: Purpose,
    wall_ms: u64,
    scope: String,
    owner_pid: u32,
    owner_birth: String,
    lock: RunVersionLock,
    cwd: PathBuf,
    args: Vec<String>,
    env: BTreeMap<String, String>,
}

pub struct ProbeOutput {
    pub stdout: Vec<u8>,
    pub stderr_empty: bool,
}

impl std::fmt::Debug for ProbeOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProbeOutput")
            .field("stdout_bytes", &self.stdout.len())
            .field("stderr_empty", &self.stderr_empty)
            .finish()
    }
}

/// Cleanup result for THIS operation only; never provider permission evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeScope {
    KnownNoChild,
    KnownSettled,
    UnknownRetained,
}
#[derive(Debug)]
pub struct ProbeFailure {
    scope: ProbeScope,
    reason: &'static str,
}
impl ProbeFailure {
    pub fn scope_known(&self) -> bool {
        !matches!(self.scope, ProbeScope::UnknownRetained)
    }
    pub fn scope(&self) -> ProbeScope {
        self.scope
    }
    pub fn reason(&self) -> &'static str {
        self.reason
    }
    fn no_child(reason: &'static str) -> Self {
        Self {
            scope: ProbeScope::KnownNoChild,
            reason,
        }
    }
}
fn finish_owned<T, O>(
    mut owner: O,
    outcome: Result<T, &'static str>,
    settle: impl FnOnce(&mut O) -> bool,
    retain: impl FnOnce(O),
) -> Result<T, ProbeFailure> {
    if !settle(&mut owner) {
        let reason = outcome.err().unwrap_or("host probe scope unsettled");
        retain(owner);
        return Err(ProbeFailure {
            scope: ProbeScope::UnknownRetained,
            reason,
        });
    }
    outcome.map_err(|reason| ProbeFailure {
        scope: ProbeScope::KnownSettled,
        reason,
    })
}
fn finish_owner<T>(mut owner: Owner, outcome: Result<T, &'static str>) -> Result<T, ProbeFailure> {
    // All callers created this exact owner before entering the fallible operation.
    let Some(inner) = owner.inner.take() else {
        return Err(ProbeFailure {
            scope: ProbeScope::UnknownRetained,
            reason: "host probe owner unavailable",
        });
    };
    finish_owned(
        inner,
        outcome,
        |inner| settle(inner, Instant::now() + Duration::from_secs(2)),
        quarantine,
    )
}

static UNKNOWN: std::sync::OnceLock<std::sync::Mutex<Vec<Inner>>> = std::sync::OnceLock::new();
pub(crate) struct HeldProcess {
    pub(crate) child: Child,
    may_execute: bool,
    _authority: Option<std::sync::Arc<dyn Send + Sync>>,
}
pub(crate) struct Inner {
    process: HeldProcess,
    pub(crate) scope: CgroupScope,
}
impl std::ops::Deref for Inner {
    type Target = HeldProcess;
    fn deref(&self) -> &HeldProcess {
        &self.process
    }
}
impl std::ops::DerefMut for Inner {
    fn deref_mut(&mut self) -> &mut HeldProcess {
        &mut self.process
    }
}

pub(crate) struct Owner {
    pub(crate) inner: Option<Inner>,
}
impl std::ops::Deref for Owner {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        self.inner.as_ref().expect("owned probe")
    }
}
impl std::ops::DerefMut for Owner {
    fn deref_mut(&mut self) -> &mut Inner {
        self.inner.as_mut().expect("owned probe")
    }
}
fn settle_owned(
    child: &mut Child,
    may_execute: bool,
    deadline: Instant,
    mut settle_scope: impl FnMut() -> bool,
) -> bool {
    let mut removed = false;
    loop {
        if !removed {
            removed = settle_scope();
        }
        if removed && !may_execute {
            // A partial attach can leave a provably blocked helper outside
            // the scope. Kill it only after the scope is proven removed.
            let _ = child.kill();
        }
        if removed && matches!(child.try_wait(), Ok(Some(_))) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
}
fn settle_held(process: &mut HeldProcess, deadline: Instant, scope: impl FnMut() -> bool) -> bool {
    settle_owned(&mut process.child, process.may_execute, deadline, scope)
}
pub(crate) fn settle(inner: &mut Inner, deadline: Instant) -> bool {
    let Inner { process, scope } = inner;
    settle_held(process, deadline, || {
        scope
            .terminate_and_remove(Duration::from_millis(50))
            .is_ok_and(|v| v.empty && v.removed)
    })
}
// Unknown owners remain strongly retained; cleanup failure never turns into a blocking wait
// or silently releases scope authority. If thread creation fails, the queue still retains them.
fn quarantine(inner: Inner) {
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };
    static ACTIVE: AtomicBool = AtomicBool::new(false);
    UNKNOWN
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(inner);
    if ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        if thread::Builder::new()
            .name("gent-host-probe-quarantine".into())
            .spawn(|| {
                loop {
                    {
                        let mut queue = UNKNOWN
                            .get()
                            .expect("probe quarantine")
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        queue.retain_mut(|inner| {
                            !settle(inner, Instant::now() + Duration::from_millis(50))
                        });
                        if queue.is_empty() {
                            ACTIVE.store(false, Ordering::Release);
                            break;
                        }
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            })
            .is_err()
        {
            ACTIVE.store(false, Ordering::Release);
        }
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        if let Some(mut inner) = self.inner.take() {
            if !settle(&mut inner, Instant::now() + Duration::from_millis(300)) {
                quarantine(inner);
            }
        }
    }
}

fn valid_env(env: &BTreeMap<String, String>) -> bool {
    env.len() <= 32
        && env.iter().all(|(key, value)| {
            matches!(
                key.as_str(),
                "HOME"
                    | "PATH"
                    | "LANG"
                    | "TMPDIR"
                    | "TEMP"
                    | "TMP"
                    | "CLAUDE_CONFIG_DIR"
                    | "CLAUDE_CODE_PROJECT_DIR_NAME"
                    | "DISABLE_AUTOUPDATER"
                    | "DISABLE_TELEMETRY"
                    | "DISABLE_ERROR_REPORTING"
                    | "CLAUDE_CODE_SKIP_PROMPT_HISTORY"
                    | "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"
                    | "CLAUDE_CODE_DISABLE_OFFICIAL_MARKETPLACE_AUTOINSTALL"
                    | "CLAUDE_CODE_DISABLE_POLICY_SKILLS"
                    | "ENABLE_CLAUDEAI_MCP_SERVERS"
                    | "CLAUDE_CODE_PLUGIN_CACHE_DIR"
                    | "CLAUDE_CODE_DEBUG_LOGS_DIR"
                    | "ANTHROPIC_BASE_URL"
            ) && value.len() <= 8192
                && !value.contains('\0')
        })
        && env.get("PATH").is_some_and(|v| v == "/usr/bin:/bin")
}

pub(crate) struct ProbeSpec<'a> {
    pub(crate) parent: &'a Path,
    pub(crate) helper: &'a RunVersionLock,
    pub(crate) provider: &'a RunVersionLock,
    pub(crate) cwd: &'a Path,
    pub(crate) args: &'a [String],
    pub(crate) env: &'a BTreeMap<String, String>,
    pub(crate) timeout: Duration,
    pub(crate) purpose: Purpose,
    pub(crate) initial_input: &'a [u8],
    pub(crate) retained_authority: Option<std::sync::Arc<dyn Send + Sync>>,
}

pub(crate) fn start(
    spec: ProbeSpec<'_>,
) -> Result<(Owner, std::process::ChildStdin, Instant), &'static str> {
    start_classified(spec).map_err(|failure| failure.reason())
}

fn start_classified(
    spec: ProbeSpec<'_>,
) -> Result<(Owner, std::process::ChildStdin, Instant), ProbeFailure> {
    let ProbeSpec {
        parent,
        helper,
        provider,
        cwd,
        args,
        env,
        timeout,
        purpose,
        initial_input,
        retained_authority,
    } = spec;
    let (scope, child, deadline) = (|| -> Result<_, &'static str> {
        if UNKNOWN
            .get()
            .is_some_and(|queue| !queue.lock().unwrap_or_else(|e| e.into_inner()).is_empty())
        {
            return Err("host probe cleanup ownership unknown");
        }
        if timeout < Duration::from_millis(100)
            || timeout > Duration::from_secs(60)
            || !allowed(purpose, provider, args, env)
            || args.len() > 128
            || args.iter().any(|v| v.len() > 16384 || v.contains('\0'))
        {
            return Err("host probe contract");
        }
        crate::immutable_image::require(Path::new(&helper.canonical_path))?;
        for lock in [helper, provider] {
            if gent_drivers::lock::rechecked_identity(lock).map_err(|_| "host image identity")?
                != *lock
            {
                return Err("host image changed");
            }
        }
        let mut random = [0u8; 16];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut random))
            .map_err(|_| "host nonce")?;
        let nonce = random.iter().fold(String::new(), |mut nonce, b| {
            use std::fmt::Write as _;
            let _ = write!(nonce, "{b:02x}");
            nonce
        });
        let limits = SandboxResourceLimits {
            max_memory_bytes: 1024 * 1024 * 1024,
            max_processes: 64,
            max_cpu_time_ms: 30_000,
        };
        let scope = CgroupScope::create(
            DelegatedParent::inspect(parent).map_err(|_| "host delegation")?,
            &nonce,
            limits,
        )
        .map_err(|_| "host scope creation")?;
        let deadline = Instant::now() + timeout;
        let child = Command::new(&helper.canonical_path)
            .arg(match purpose {
                Purpose::ClaudeCredentialProbe => "--host-credential-probe",
                Purpose::GentMcpStdio => "--trusted-host-mcp",
                Purpose::NodeMcpStdio => "--trusted-node-mcp",
            })
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(match purpose {
                Purpose::ClaudeCredentialProbe => Stdio::piped(),
                Purpose::GentMcpStdio | Purpose::NodeMcpStdio => Stdio::null(),
            })
            .spawn()
            .map_err(|_| "host helper spawn")?;
        Ok((scope, child, deadline))
    })()
    .map_err(ProbeFailure::no_child)?;
    let mut owner = Owner {
        inner: Some(Inner {
            process: HeldProcess {
                child,
                may_execute: false,
                _authority: retained_authority,
            },
            scope,
        }),
    };
    let operation = (|| -> Result<_, &'static str> {
        let birth = crate::native_runtime::birth(owner.child.id())?;
        {
            let inner = owner.inner.as_mut().ok_or("host owner absent")?;
            let Inner { process, scope } = inner;
            scope
                .attach_blocked_helper(&mut process.child, &birth)
                .map_err(|_| "host scope attach")?;
        }
        let start = Start {
            schema: 2,
            purpose,
            wall_ms: deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .try_into()
                .map_err(|_| "host wall bound")?,
            scope: owner
                .scope
                .membership_path()
                .map_err(|_| "host scope identity")?,
            owner_pid: std::process::id(),
            owner_birth: crate::native_runtime::birth(std::process::id())?,
            lock: provider.clone(),
            cwd: cwd.to_owned(),
            args: args.to_vec(),
            env: env.clone(),
        };
        let mut payload = serde_json::to_vec(&start).map_err(|_| "host probe encoding")?;
        if payload.len() > 65535 {
            return Err("host probe input bound");
        }
        payload.push(b'\n');
        if initial_input.len() > 16384
            || (!matches!(purpose, Purpose::GentMcpStdio | Purpose::NodeMcpStdio)
                && !initial_input.is_empty())
        {
            return Err("host initial frame rejected");
        }
        payload.extend_from_slice(initial_input);
        let mut input = owner.child.stdin.take().ok_or("host probe stdin")?;
        // Arm BEFORE the first write: a complete frame can immediately exec.
        owner.may_execute = true;
        bounded_startup(&mut input, &payload, deadline, || {
            owner.scope.sample_cpu_budget().is_ok_and(|v| !v.exceeded)
        })?;
        Ok((input, deadline))
    })();
    match operation {
        Ok((input, deadline)) => Ok((owner, input, deadline)),
        Err(reason) => finish_owner(owner, Err(reason)),
    }
}

pub fn run(
    parent: &Path,
    helper: &RunVersionLock,
    provider: &RunVersionLock,
    cwd: &Path,
    args: &[String],
    env: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<ProbeOutput, &'static str> {
    run_classified(parent, helper, provider, cwd, args, env, timeout)
        .map_err(|failure| failure.reason())
}

pub fn run_classified(
    parent: &Path,
    helper: &RunVersionLock,
    provider: &RunVersionLock,
    cwd: &Path,
    args: &[String],
    env: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<ProbeOutput, ProbeFailure> {
    let (mut owner, input, deadline) = start_classified(ProbeSpec {
        parent,
        helper,
        provider,
        cwd,
        args,
        env,
        timeout,
        purpose: Purpose::ClaudeCredentialProbe,
        initial_input: &[],
        retained_authority: None,
    })?;
    let operation = (|| -> Result<_, &'static str> {
        drop(input);
        let mut output = owner.child.stdout.take().ok_or("host probe stdout")?;
        let mut error = owner.child.stderr.take().ok_or("host probe stderr")?;
        for fd in [
            &output as &dyn std::os::fd::AsFd,
            &error as &dyn std::os::fd::AsFd,
        ] {
            let flags = rustix::fs::fcntl_getfl(fd).map_err(|_| "host pipe flags")?;
            rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK)
                .map_err(|_| "host pipe nonblocking")?;
        }
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut success = false;
        while Instant::now() < deadline {
            if drain(&mut output, &mut out).is_err() || drain(&mut error, &mut err).is_err() {
                break;
            }
            if owner.scope.sample_cpu_budget().map_or(true, |v| v.exceeded) {
                break;
            }
            match owner.child.try_wait() {
                Ok(Some(status)) => {
                    success = status.success();
                    break;
                }
                Ok(None) => thread::sleep(Duration::from_millis(5)),
                Err(_) => break,
            }
        }
        Ok((success, output, error, out, err))
    })();
    let (success, mut output, mut error, mut out, mut err) = finish_owner(owner, operation)?;
    drain(&mut output, &mut out).map_err(|reason| ProbeFailure {
        scope: ProbeScope::KnownSettled,
        reason,
    })?;
    drain(&mut error, &mut err).map_err(|reason| ProbeFailure {
        scope: ProbeScope::KnownSettled,
        reason,
    })?;
    if !success || out.len() > 65536 || err.len() > 65536 {
        return Err(ProbeFailure {
            scope: ProbeScope::KnownSettled,
            reason: "host probe failed or output bound",
        });
    }
    Ok(ProbeOutput {
        stdout: out,
        stderr_empty: err.is_empty(),
    })
}

fn bounded_startup(
    input: &mut std::process::ChildStdin,
    payload: &[u8],
    deadline: Instant,
    current: impl Fn() -> bool,
) -> Result<(), &'static str> {
    let flags = rustix::fs::fcntl_getfl(&*input).map_err(|_| "host startup pipe flags")?;
    rustix::fs::fcntl_setfl(&*input, flags | rustix::fs::OFlags::NONBLOCK)
        .map_err(|_| "host startup nonblocking")?;
    let mut offset = 0;
    while offset < payload.len() {
        if Instant::now() >= deadline || !current() {
            return Err("host startup deadline or CPU budget");
        }
        match input.write(&payload[offset..]) {
            Ok(0) => return Err("host startup closed"),
            Ok(n) => offset += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2))
            }
            Err(_) => return Err("host startup write"),
        }
    }
    Ok(())
}

fn drain(reader: &mut impl Read, bytes: &mut Vec<u8>) -> Result<(), &'static str> {
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                bytes.extend_from_slice(&buf[..n]);
                if bytes.len() > 65536 {
                    return Err("host output bound");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Err("host output read"),
        }
    }
}

/// The private helper entry point; no SDK runs until cgroup/owner checks succeed.
fn retained_scope(scope: &str) -> Result<(std::fs::File, std::fs::File), &'static str> {
    use rustix::fs::{CWD, Mode, OFlags, ResolveFlags, openat2};
    let scope = std::fs::File::from(
        openat2(
            CWD,
            format!("/sys/fs/cgroup{scope}"),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS,
        )
        .map_err(|_| "host retained scope")?,
    );
    let kill = std::fs::File::from(
        openat2(
            &scope,
            "cgroup.kill",
            OFlags::WRONLY | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS,
        )
        .map_err(|_| "host retained tree kill")?,
    );
    Ok((scope, kill))
}

fn spawn_owner_monitor(
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    start: &Start,
    mut kill: std::fs::File,
    begun: Instant,
) -> Result<thread::JoinHandle<()>, &'static str> {
    let owner_pid = start.owner_pid;
    let owner_birth = start.owner_birth.clone();
    let wall_ms = start.wall_ms;
    thread::Builder::new()
        .name("gent-host-owner".into())
        .spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Acquire) {
                if rustix::process::getppid().map(|p| p.as_raw_pid().cast_unsigned())
                    != Some(owner_pid)
                    || crate::native_runtime::birth(owner_pid).ok().as_deref()
                        != Some(owner_birth.as_str())
                    || begun.elapsed() >= Duration::from_millis(wall_ms)
                {
                    kill_retaining(&mut kill);
                }
                thread::sleep(Duration::from_millis(5));
            }
        })
        .map_err(|_| "host owner monitor spawn")
}

fn require_only_self(scope: &std::fs::File, kill: &mut std::fs::File) {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    let mut procs = String::new();
    let mut file = std::fs::File::from(
        openat2(
            scope,
            "cgroup.procs",
            OFlags::RDONLY | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS,
        )
        .unwrap_or_else(|_| kill_retaining(kill)),
    );
    Read::by_ref(&mut file)
        .take(8193)
        .read_to_string(&mut procs)
        .unwrap_or_else(|_| kill_retaining(kill));
    if procs.len() > 8192
        || procs
            .lines()
            .any(|p| p.parse::<u32>().ok() != Some(std::process::id()))
    {
        kill_retaining(kill);
    }
}

fn kill_retaining(kill: &mut std::fs::File) -> ! {
    loop {
        let _ = kill.write_all(b"1");
        thread::sleep(Duration::from_millis(50));
    }
}

struct HostMonitor {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    data: Option<std::fs::File>,
}
impl Drop for HostMonitor {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        // A failed kill intentionally retains this monitor inside the helper.
        // The API caller's bounded owner then quarantines the whole cgroup.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
pub fn guest() -> Result<i32, &'static str> {
    guest_for(Purpose::ClaudeCredentialProbe)
}
pub fn guest_node_mcp() -> Result<i32, &'static str> {
    guest_for(Purpose::NodeMcpStdio)
}
pub fn guest_mcp() -> Result<i32, &'static str> {
    guest_for(Purpose::GentMcpStdio)
}
fn guest_for(purpose: Purpose) -> Result<i32, &'static str> {
    let begun = Instant::now();
    // Stdin's internal BufReader could consume the first pipelined MCP frame.
    // The private Start is read through a raw duplicated descriptor only.
    let mut private_input =
        std::fs::File::from(rustix::io::dup(std::io::stdin()).map_err(|_| "host private input")?);
    let start: Start =
        serde_json::from_slice(&crate::contained_gate::line(&mut private_input, 65536)?)
            .map_err(|_| "host startup contract")?;
    drop(private_input);
    let owner_current = || {
        rustix::process::getppid().map(|v| v.as_raw_pid().cast_unsigned()) == Some(start.owner_pid)
            && crate::native_runtime::birth(start.owner_pid)
                .ok()
                .as_deref()
                == Some(start.owner_birth.as_str())
    };
    if start.schema != 2
        || start.purpose != purpose
        || start.wall_ms == 0
        || start.wall_ms > 60_000
        || !allowed(purpose, &start.lock, &start.args, &start.env)
        || (purpose == Purpose::NodeMcpStdio
            && start.env.get("HOME").map(PathBuf::from).as_ref() != Some(&start.cwd))
        || !owner_current()
        || std::fs::read_to_string("/proc/self/cgroup")
            .map_err(|_| "host scope unavailable")?
            .trim()
            != format!("0::{}", start.scope)
        || !start.scope.starts_with('/')
        || start.scope.split('/').any(|p| matches!(p, "." | ".."))
    {
        return Err("host owner or scope changed");
    }
    let (scope, mut kill) = retained_scope(&start.scope)?;
    let monitor_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let monitor = spawn_owner_monitor(
        monitor_stop.clone(),
        &start,
        kill.try_clone().map_err(|_| "host monitor kill handle")?,
        begun,
    )?;
    let (data_pin, arguments) = if purpose == Purpose::GentMcpStdio {
        if !owner_current() {
            kill_retaining(&mut kill);
        }
        let (pin, arguments) =
            crate::host_stdio::retain_data_directory(&start.args, start.owner_pid)?;
        if !owner_current() {
            kill_retaining(&mut kill);
        }
        (Some(pin), arguments)
    } else if purpose == Purpose::NodeMcpStdio {
        if !owner_current() {
            kill_retaining(&mut kill);
        }
        let (pin, arguments) = crate::node_entry::capture(&start.args)?;
        if !owner_current() {
            kill_retaining(&mut kill);
        }
        (Some(pin), arguments)
    } else {
        (None, start.args.clone())
    };
    let monitor = HostMonitor {
        stop: monitor_stop,
        thread: Some(monitor),
        data: data_pin,
    };
    nix::sys::prctl::set_no_new_privs().map_err(|_| "host no-new-privileges")?;
    let snapshot = crate::sealed_exec::SealedExecutable::capture(&start.lock)?;
    if gent_drivers::lock::rechecked_identity(&start.lock).map_err(|_| "host executable")?
        != start.lock
    {
        return Err("host executable changed");
    }
    if !owner_current() || begun.elapsed() >= Duration::from_millis(start.wall_ms) {
        kill_retaining(&mut kill);
    }
    let mut command = snapshot.host_command()?;
    let inheritance = monitor
        .data
        .as_ref()
        .map(crate::host_stdio::DataInheritance::new)
        .transpose()?;
    let spawned = command
        .args(&arguments)
        .current_dir(&start.cwd)
        .env_clear()
        .envs(&start.env)
        .spawn();
    if inheritance.is_some_and(|guard| guard.restore().is_err()) {
        // A child may already exist. Retain independent kill authority rather
        // than returning an ambiguous spawn error and dropping the watchdog.
        kill_retaining(&mut kill);
    }
    let mut child = spawned.map_err(|_| "host SDK spawn")?;
    let witness_deadline =
        (Instant::now() + Duration::from_secs(3)).min(begun + Duration::from_millis(start.wall_ms));
    let mut witnessed = false;
    while Instant::now() < witness_deadline {
        if snapshot
            .host_executable_matches(child.id())
            .unwrap_or(false)
        {
            witnessed = true;
            break;
        }
        if !owner_current() || matches!(child.try_wait(), Ok(Some(_)) | Err(_)) {
            kill_retaining(&mut kill);
        }
        thread::sleep(Duration::from_millis(1));
    }
    if !witnessed {
        kill_retaining(&mut kill);
    }
    loop {
        // Retained helper survives the original owner and owns the exact whole-tree kill FD.
        // It remains in this same cgroup; cgroup.kill deliberately terminates the helper too.
        if !owner_current() || begun.elapsed() >= Duration::from_millis(start.wall_ms) {
            kill_retaining(&mut kill);
        }
        if snapshot.source_current().is_err() {
            kill_retaining(&mut kill);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                require_only_self(&scope, &mut kill);
                return Ok(status.code().unwrap_or(125));
            }
            Ok(None) => thread::sleep(Duration::from_millis(5)),
            Err(_) => {
                kill_retaining(&mut kill);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_start_and_unknown_cleanup_retain_same_process_authority() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        struct TicketDrop(Arc<AtomicUsize>);
        impl Drop for TicketDrop {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::AcqRel);
            }
        }
        struct Authority {
            _directory: tempfile::TempDir,
            _pin: std::fs::File,
            _ticket: TicketDrop,
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_owned();
        let pin = std::fs::File::create(path.join("authority.pin")).unwrap();
        let dropped = Arc::new(AtomicUsize::new(0));
        let authority = Arc::new(Authority {
            _directory: directory,
            _pin: pin,
            _ticket: TicketDrop(dropped.clone()),
        });
        let weak = Arc::downgrade(&authority);
        let child = Command::new("/usr/bin/sleep")
            .arg("20")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut held = HeldProcess {
            child,
            may_execute: true,
            _authority: Some(authority.clone()),
        };
        let mut input = held.child.stdin.take().unwrap();
        nix::fcntl::fcntl(&input, nix::fcntl::FcntlArg::F_SETPIPE_SZ(4096)).unwrap();
        let barriers = std::cell::Cell::new(0);
        let partial = bounded_startup(
            &mut input,
            &vec![b'x'; 8192],
            Instant::now() + Duration::from_secs(1),
            || {
                barriers.set(barriers.get() + 1);
                barriers.get() < 2
            },
        );
        drop(input);
        drop(authority);
        // Only the scope operation is injected to fail. This is not evidence
        // of a kernel cgroup failure; the partial write and owned child are real.
        let unknown = !settle_held(
            &mut held,
            Instant::now() + Duration::from_millis(20),
            || false,
        );
        let alive = matches!(held.child.try_wait(), Ok(None));
        let retained =
            weak.upgrade().is_some() && dropped.load(Ordering::Acquire) == 0 && path.exists();
        // Exact owner transfer (the same HeldProcess used inside production
        // Inner/quarantine), not inference from any global queue's emptiness.
        let mut quarantine = vec![held];
        let _ = quarantine[0].child.kill();
        let _ = quarantine[0].child.wait();
        let known = settle_held(
            &mut quarantine[0],
            Instant::now() + Duration::from_secs(1),
            || true,
        );
        drop(quarantine);
        assert!(partial.is_err() && barriers.get() == 2 && unknown && alive && retained);
        assert!(
            known
                && dropped.load(Ordering::Acquire) == 1
                && weak.upgrade().is_none()
                && !path.exists()
        );
    }
    #[test]
    fn failed_tree_kill_keeps_helper_alive_with_its_authority() {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "host_probe::tests::kill_failure_child"])
            .env_clear()
            .env("GENT_QA_KILL_FAILURE_CHILD", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        thread::sleep(Duration::from_millis(100));
        let retained = matches!(child.try_wait(), Ok(None));
        let _ = child.kill();
        let until = Instant::now() + Duration::from_secs(1);
        let mut reaped = false;
        while Instant::now() < until {
            if matches!(child.try_wait(), Ok(Some(_))) {
                reaped = true;
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            retained,
            "failed kill must not make the helper relinquish ownership"
        );
        assert!(reaped);
    }
    #[test]
    fn kill_failure_child() {
        if std::env::var("GENT_QA_KILL_FAILURE_CHILD").as_deref() == Ok("1") {
            // Real ENOSPC on an owned FD exercises the production failure loop;
            // it is deliberately not evidence of a failing cgroup kernel.
            let mut fail = std::fs::OpenOptions::new()
                .write(true)
                .open("/dev/full")
                .unwrap();
            kill_retaining(&mut fail);
        }
    }
    #[test]
    fn nonreading_startup_pipe_cannot_escape_deadline() {
        let mut child = Command::new("/usr/bin/sleep")
            .arg("20")
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        let begun = Instant::now();
        let result = bounded_startup(
            &mut input,
            &vec![b'x'; 2 * 1024 * 1024],
            begun + Duration::from_millis(50),
            || true,
        );
        let elapsed = begun.elapsed();
        let _ = child.kill();
        let _ = child.wait();
        assert!(result.is_err());
        assert!(elapsed < Duration::from_secs(1));
    }
    #[test]
    fn failed_scope_settlement_preserves_independent_released_helper() {
        // Inject only the scope-operation failure. The child is a real owned
        // process; this is not evidence of a failing cgroup kernel.
        let mut helper = Command::new("/usr/bin/sleep")
            .arg("20")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let begun = Instant::now();
        let settled = settle_owned(&mut helper, true, begun + Duration::from_millis(40), || {
            false
        });
        let alive = matches!(helper.try_wait(), Ok(None));
        let _ = helper.kill();
        let _ = helper.wait();
        assert!(!settled);
        assert!(
            alive,
            "scope failure must not kill the released watchdog helper"
        );
        assert!(begun.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn blocked_prestart_helper_can_be_reaped_without_scope_membership() {
        let mut helper = Command::new("/usr/bin/sleep")
            .arg("20")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let settled = settle_owned(
            &mut helper,
            false,
            Instant::now() + Duration::from_secs(1),
            || true,
        );
        let reaped = matches!(helper.try_wait(), Ok(Some(_)));
        if !reaped {
            let _ = helper.kill();
            let _ = helper.wait();
        }
        assert!(settled && reaped);
    }
    #[test]
    fn loader_and_credential_injection_are_not_ambient_environment() {
        let mut env = BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]);
        assert!(valid_env(&env));
        for key in [
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "NODE_OPTIONS",
            "PYTHONPATH",
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
        ] {
            env.insert(key.into(), "canary".into());
            assert!(!valid_env(&env));
            env.remove(key);
        }
    }
}

#[cfg(test)]
mod typed_failure_tests {
    use super::*;
    #[test]
    fn no_spawn_failure_is_known_without_executable_access() {
        let lock = RunVersionLock {
            provider: "claude".into(),
            canonical_path: "/definitely-not-a-real-sdk".into(),
            file_identity: "unused".into(),
            digest_sha256: "a".repeat(64),
            version: "unused".into(),
            compatibility_entry: "unused".into(),
        };
        let failure = run_classified(
            Path::new("/not-a-cgroup"),
            &lock,
            &lock,
            Path::new("/"),
            &[],
            &BTreeMap::new(),
            Duration::ZERO,
        )
        .unwrap_err();
        assert_eq!(failure.scope(), ProbeScope::KnownNoChild);
        assert!(failure.scope_known());
    }
    #[test]
    fn known_settled_nonzero_failure_does_not_latch_unknown() {
        // Exact real child exit, injected scope-success primitive; no kernel witness claim.
        let child = Command::new("/usr/bin/false")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let result: Result<(), ProbeFailure> = finish_owned(
            child,
            Err("host probe failed or output bound"),
            |child| {
                settle_owned(child, true, Instant::now() + Duration::from_secs(1), || {
                    true
                })
            },
            |_| panic!("known child must not be quarantined"),
        );
        let failure = result.unwrap_err();
        assert!(failure.scope_known());
        assert_eq!(failure.scope(), ProbeScope::KnownSettled);
    }
    #[test]
    fn unknown_failure_retains_exact_alive_child_and_authority() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        struct Authority(Arc<AtomicUsize>);
        impl Drop for Authority {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::AcqRel);
            }
        }
        let dropped = Arc::new(AtomicUsize::new(0));
        let authority = Arc::new(Authority(dropped.clone()));
        let weak = Arc::downgrade(&authority);
        let process = HeldProcess {
            child: Command::new("/usr/bin/sleep")
                .arg("20")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
            may_execute: true,
            _authority: Some(authority),
        };
        let parked = std::cell::RefCell::new(None);
        // Inject only settlement failure; the same real process+authority transfer is exercised.
        let result: Result<(), ProbeFailure> = finish_owned(
            process,
            Err("host startup write"),
            |_| false,
            |process| {
                parked.replace(Some(process));
            },
        );
        let failure = result.unwrap_err();
        let mut process = parked.take().unwrap();
        let alive = matches!(process.child.try_wait(), Ok(None));
        let retained = weak.upgrade().is_some() && dropped.load(Ordering::Acquire) == 0;
        let _ = process.child.kill();
        let _ = process.child.wait();
        drop(process);
        assert!(!failure.scope_known());
        assert_eq!(failure.scope(), ProbeScope::UnknownRetained);
        assert!(alive && retained);
        assert_eq!(dropped.load(Ordering::Acquire), 1);
    }
}
