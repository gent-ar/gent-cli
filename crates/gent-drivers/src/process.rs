//! The operating-system edge for public provider processes.
//!
//! It starts only lock-validated Claude or Codex executables. Output collection is bounded at
//! the reader edge so an uncooperative provider cannot grow driver memory without limit.

use std::io::{Read, Result as IoResult, Write};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};

use crate::interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal};
use crate::supervisor::{ProcessLauncher, ProviderLaunch, ProviderProcess, SupervisorError};

/// Bounded capture from one provider stream.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CapturedStream {
    pub bytes: Vec<u8>,
    pub discarded_bytes: usize,
}

/// Bounded stdout and stderr captured from a public provider process.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProcessOutput {
    pub stdout: CapturedStream,
    pub stderr: CapturedStream,
}

/// A synchronous launcher for public executables with a fixed per-stream output limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SystemLauncher {
    output_limit: usize,
}

impl SystemLauncher {
    #[must_use]
    pub const fn new(output_limit: usize) -> Self {
        Self { output_limit }
    }
}

impl ProcessLauncher for SystemLauncher {
    type Process = SystemProcess;

    fn launch(&self, launch: &ProviderLaunch) -> Result<SystemProcess, SupervisorError> {
        validate_public_provider(&launch.provider)?;
        let mut command = Command::new(&launch.executable);
        command
            .args(&launch.arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_process_tree(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| SupervisorError::Launch(error.to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SupervisorError::Launch("stdout was not piped".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| SupervisorError::Launch("stderr was not piped".into()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| SupervisorError::Launch("stdin was not piped".into()))?;
        Ok(SystemProcess::new(
            child,
            stdin,
            stdout,
            stderr,
            self.output_limit,
        ))
    }
}

/// A provider-owned process group with bounded asynchronous pipe readers.
#[derive(Debug)]
pub struct SystemProcess {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    output: Arc<Mutex<ProcessOutput>>,
    readers: Mutex<Vec<JoinHandle<()>>>,
}

impl SystemProcess {
    fn new(
        child: Child,
        stdin: ChildStdin,
        stdout: impl Read + Send + 'static,
        stderr: impl Read + Send + 'static,
        output_limit: usize,
    ) -> Self {
        let output = Arc::new(Mutex::new(ProcessOutput::default()));
        let readers = vec![
            start_reader(stdout, &output, output_limit, StreamKind::Stdout),
            start_reader(stderr, &output, output_limit, StreamKind::Stderr),
        ];
        Self {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            output,
            readers: Mutex::new(readers),
        }
    }

    /// Waits for process exit and joins both pipe readers before returning its exit status.
    ///
    /// # Errors
    /// Returns an error when waiting for the operating-system process fails.
    pub fn wait(&self) -> IoResult<ExitStatus> {
        let status = recover_lock(&self.child).wait()?;
        for reader in recover_lock(&self.readers).drain(..) {
            let _ = reader.join();
        }
        Ok(status)
    }

    /// Returns a consistent snapshot of the bounded captures collected so far.
    #[must_use]
    pub fn output(&self) -> ProcessOutput {
        recover_lock(&self.output).clone()
    }
}

impl ProcessTreeControl for SystemProcess {
    fn signal_tree(&self, signal: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        let pid = self
            .child
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .id()
            .try_into()
            .map_err(|_| ProcessTreeError::Failed("provider process id overflowed".into()))?;
        signal_process_tree(pid, signal)
    }
}

impl ProviderProcess for SystemProcess {
    fn write_frame(&self, frame: &[u8]) -> Result<(), ProcessTreeError> {
        let mut stdin = recover_lock(&self.stdin);
        stdin
            .write_all(frame)
            .and_then(|()| stdin.flush())
            .map_err(|error| ProcessTreeError::Failed(error.to_string()))
    }
}

fn validate_public_provider(provider: &str) -> Result<(), SupervisorError> {
    matches!(provider, "claude" | "codex")
        .then_some(())
        .ok_or_else(|| SupervisorError::UnsupportedProvider(provider.into()))
}

#[cfg(unix)]
fn configure_process_tree(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_process_tree(_: &mut Command) {}

#[cfg(unix)]
fn signal_process_tree(pid: i32, signal: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
    let name = match signal {
        ProcessTreeSignal::Interrupt => "INT",
        ProcessTreeSignal::Terminate => "TERM",
        ProcessTreeSignal::Kill => "KILL",
    };
    // A negative pid targets the group created by `configure_process_tree`. Using the platform
    // utility keeps this workspace free of unsafe FFI while retaining whole-group semantics.
    let status = Command::new("/bin/kill")
        .args(["-s", name, "--", &format!("-{pid}")])
        .status()
        .map_err(|error| ProcessTreeError::Failed(error.to_string()))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| ProcessTreeError::Failed(format!("kill exited with {status}")))
}

#[cfg(not(unix))]
fn signal_process_tree(_: i32, _: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
    Err(ProcessTreeError::Failed(
        "process-tree signaling is not implemented on this platform".into(),
    ))
}

#[derive(Clone, Copy)]
enum StreamKind {
    Stdout,
    Stderr,
}

fn start_reader(
    reader: impl Read + Send + 'static,
    output: &Arc<Mutex<ProcessOutput>>,
    limit: usize,
    stream: StreamKind,
) -> JoinHandle<()> {
    let output = Arc::clone(output);
    thread::spawn(move || read_bounded(reader, &output, limit, stream))
}

fn read_bounded(
    mut reader: impl Read,
    output: &Arc<Mutex<ProcessOutput>>,
    limit: usize,
    stream: StreamKind,
) {
    let mut buffer = [0_u8; 4096];
    while let Ok(read) = reader.read(&mut buffer) {
        if read == 0 {
            return;
        }
        append(&mut recover_lock(output), &buffer[..read], limit, stream);
    }
}

fn recover_lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn append(output: &mut ProcessOutput, chunk: &[u8], limit: usize, stream: StreamKind) {
    let target = match stream {
        StreamKind::Stdout => &mut output.stdout,
        StreamKind::Stderr => &mut output.stderr,
    };
    let accepted = limit.saturating_sub(target.bytes.len()).min(chunk.len());
    target.bytes.extend_from_slice(&chunk[..accepted]);
    target.discarded_bytes += chunk.len() - accepted;
}
