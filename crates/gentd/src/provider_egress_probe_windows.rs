use super::{
    BTreeMap, Child, Duration, File, Instant, ProbeFailure, ProbeOutput, RunVersionLock, Stdio,
    await_with_authority, bounded, decode, rechecked_identity, thread,
};
#[cfg(test)]
type EventLog = std::sync::Arc<std::sync::Mutex<Vec<(&'static str, bool)>>>;
#[derive(Clone, Default)]
struct Observer {
    #[cfg(test)]
    events: Option<EventLog>,
}
impl Observer {
    fn record(&self, event: &'static str, known: bool) {
        #[cfg(test)]
        if let Some(events) = &self.events {
            events.lock().unwrap().push((event, known));
        }
        #[cfg(not(test))]
        let _ = (self, event, known);
    }
}
fn pin(lock: &RunVersionLock) -> Result<File, ProbeFailure> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&lock.canonical_path)
        .map_err(|_| ProbeFailure::known())?;
    let metadata = file.metadata().map_err(|_| ProbeFailure::known())?;
    if !metadata.is_file()
        || metadata.file_attributes() & 0x400 != 0
        || rechecked_identity(lock).map_err(|_| ProbeFailure::known())? != *lock
    {
        return Err(ProbeFailure::known());
    }
    Ok(file)
}
struct NativeOwner {
    child: Option<Child>,
    control_stdin: Option<std::process::ChildStdin>,
    _pins: std::sync::Arc<Vec<File>>,
    known: bool,
    observer: Observer,
}
impl Drop for NativeOwner {
    fn drop(&mut self) {
        let control = self.control_stdin.take();
        let held = control.is_some();
        drop(control);
        if held {
            self.observer.record("control-closed", self.known);
        }
        if !self.known {
            if let Some(child) = self.child.as_mut() {
                let _ = child.kill();
            }
        }
    }
}
pub(super) fn run_with_helper(
    helper: RunVersionLock,
    lock: RunVersionLock,
    cwd: std::path::PathBuf,
    args: Vec<String>,
    environment: BTreeMap<String, String>,
    timeout: Duration,
) -> Result<ProbeOutput, ProbeFailure> {
    run_observed(
        helper,
        lock,
        cwd,
        args,
        environment,
        timeout,
        Observer::default(),
    )
}
fn run_observed(
    helper: RunVersionLock,
    lock: RunVersionLock,
    cwd: std::path::PathBuf,
    args: Vec<String>,
    environment: BTreeMap<String, String>,
    timeout: Duration,
    observer: Observer,
) -> Result<ProbeOutput, ProbeFailure> {
    if timeout.is_zero() || timeout > Duration::from_secs(45) {
        return Err(ProbeFailure::known());
    }
    let pins = std::sync::Arc::new(vec![pin(&helper)?, pin(&lock)?]);
    let worker_pins = pins.clone();
    let deadline = Instant::now() + timeout + Duration::from_secs(12);
    let worker = thread::Builder::new()
        .name("credential-probe-owner".into())
        .spawn(move || {
            let mut command = gent_types::hidden_command(&helper.canonical_path);
            command
                .arg("host-credential-probe")
                .arg(&lock.canonical_path)
                .arg(&lock.digest_sha256)
                .arg(&cwd)
                .arg(timeout.as_millis().to_string())
                .arg("--")
                .args(args)
                .env_clear()
                .envs(environment)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let child = command.spawn().map_err(|_| ProbeFailure::known())?;
            let mut owner = NativeOwner {
                child: Some(child),
                control_stdin: None,
                _pins: worker_pins,
                known: false,
                observer: observer.clone(),
            };
            let Some(child) = owner.child.as_mut() else {
                return Err(ProbeFailure::retained(owner));
            };
            // Child::wait closes child.stdin. This separate exact capability
            // must stay alive through native Job0/drain0, not signal death.
            owner.control_stdin = child.stdin.take();
            if owner.control_stdin.is_none() {
                return Err(ProbeFailure::retained(owner));
            }
            owner.observer.record("control-held", false);
            let Some(stdout) = child.stdout.take() else {
                return Err(ProbeFailure::retained(owner));
            };
            let Some(stderr) = child.stderr.take() else {
                return Err(ProbeFailure::retained(owner));
            };
            let Ok(output) = thread::Builder::new()
                .name("probe-result".into())
                .spawn(move || bounded(stdout, 65556))
            else {
                return Err(ProbeFailure::retained(owner));
            };
            let Ok(error) = thread::Builder::new()
                .name("probe-error-drain".into())
                .spawn(move || bounded(stderr, 4096))
            else {
                return Err(ProbeFailure::retained(owner));
            };
            let status = match owner.child.as_mut() {
                Some(value) => value.wait(),
                None => return Err(ProbeFailure::retained(owner)),
            };
            // This worker may be retained, but the calling thread never joins a
            // blocked helper/drain: its deadline returns a tracked Unknown owner.
            let output = output.join();
            let error = error.join();
            let Ok(Ok(bytes)) = output else {
                return Err(ProbeFailure::retained(owner));
            };
            if !matches!(status,Ok(status) if status.success()) {
                return Err(ProbeFailure::retained(owner));
            }
            let Some(decoded) = decode(&bytes) else {
                return Err(ProbeFailure::retained(owner));
            };
            owner.known = true;
            owner.observer.record("scope-known", true); // Native frame authenticates exact child Job0/drain0.
            if !matches!(error,Ok(Ok(bytes)) if bytes.is_empty())
                || rechecked_identity(&helper).ok().as_ref() != Some(&helper)
                || rechecked_identity(&lock).ok().as_ref() != Some(&lock)
            {
                return Err(ProbeFailure::known());
            }
            decoded
        })
        .map_err(|_| ProbeFailure::known())?;
    await_with_authority(worker, deadline, pins)
}

#[cfg(test)]
#[path = "provider_egress_probe_writer_tests.rs"]
mod writer_tests;
