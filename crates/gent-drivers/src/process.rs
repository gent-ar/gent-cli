use std::collections::VecDeque;
use std::io::{Read, Result as IoResult, Write};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use crate::interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal};
use crate::lock::rechecked_identity;
use crate::process_streams::ProcessStreams;
pub use crate::process_streams::{CapturedStream, ProcessOutput};
use crate::supervisor::{ProcessLauncher, ProviderLaunch, ProviderProcess, SupervisorError};
use node_search::NodeSearch;
pub use node_search::configure_locked_node_environment;

/// A synchronous launcher for public executables with a fixed per-stream output limit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SystemLauncher {
    output_limit: usize,
    node: NodeSearch,
}

impl SystemLauncher {
    #[must_use]
    pub const fn new(output_limit: usize) -> Self {
        Self {
            output_limit,
            node: NodeSearch::Inherited,
        }
    }

    /// Puts a caller-locked Node directory first for npm-installed provider shims.
    #[must_use]
    pub fn with_preferred_node(output_limit: usize, node_bin: std::path::PathBuf) -> Self {
        Self {
            output_limit,
            node: NodeSearch::Locked(node_bin),
        }
    }

    #[must_use]
    pub const fn with_node_first(output_limit: usize, node_bin: std::path::PathBuf) -> Self {
        Self {
            output_limit,
            node: NodeSearch::First(node_bin),
        }
    }
}

impl ProcessLauncher for SystemLauncher {
    type Process = SystemProcess;

    fn launch(&self, launch: &ProviderLaunch) -> Result<SystemProcess, SupervisorError> {
        validate_public_provider(&launch.provider)?;
        (rechecked_identity(&launch.lock)? == launch.lock)
            .then_some(())
            .ok_or(SupervisorError::Lock(
                crate::lock::LockError::ProviderChanged,
            ))?;
        (launch.executable.to_string_lossy() == launch.lock.canonical_path
            && launch.provider == launch.lock.provider)
            .then_some(())
            .ok_or(SupervisorError::Lock(
                crate::lock::LockError::ProviderChanged,
            ))?;
        let mut command = Command::new(&launch.executable);
        command
            .args(&launch.arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        self.node.configure(&mut command)?;
        node_search::remove_package_manager_provenance(&mut command);
        if let Some(workspace_root) = &launch.workspace_root {
            command.current_dir(workspace_root);
        }
        configure_process_tree(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| SupervisorError::Launch(error.to_string()))?;
        groups::spawned(child.id());
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

#[derive(Debug)]
pub struct SystemProcess {
    child: Mutex<Child>,
    stdin: Mutex<Option<ChildStdin>>,
    streams: ProcessStreams,
    drained_stdout: Mutex<VecDeque<Vec<u8>>>,
}

impl SystemProcess {
    fn new(
        child: Child,
        stdin: ChildStdin,
        stdout: impl Read + Send + 'static,
        stderr: impl Read + Send + 'static,
        output_limit: usize,
    ) -> Self {
        Self {
            child: Mutex::new(child),
            stdin: Mutex::new(Some(stdin)),
            streams: ProcessStreams::new(stdout, stderr, output_limit),
            drained_stdout: Mutex::new(VecDeque::new()),
        }
    }

    /// Waits for process exit and joins both pipe readers before returning its exit status.
    ///
    /// # Errors
    /// Returns an error when waiting for the operating-system process fails.
    pub fn wait(&self) -> IoResult<ExitStatus> {
        loop {
            if let Some(status) = self.try_wait_status()? {
                return Ok(status);
            }
            let _ = self.streams.next_stdout_chunk();
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[must_use]
    pub fn output(&self) -> ProcessOutput {
        self.streams.output()
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
        let input = stdin
            .as_mut()
            .ok_or_else(|| ProcessTreeError::Failed("provider input is closed".into()))?;
        input
            .write_all(frame)
            .and_then(|()| input.flush())
            .map_err(|error| ProcessTreeError::Failed(error.to_string()))
    }

    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
        recover_lock(&self.stdin).take();
        Ok(())
    }

    fn next_stdout_chunk(&self) -> Result<Option<Vec<u8>>, ProcessTreeError> {
        Ok(self
            .streams
            .next_stdout_chunk()
            .or_else(|| recover_lock(&self.drained_stdout).pop_front()))
    }

    fn try_exit_code(&self) -> Result<Option<Option<i32>>, ProcessTreeError> {
        self.try_wait_status()
            .map(|status| status.map(|status| status.code()))
            .map_err(|error| ProcessTreeError::Failed(error.to_string()))
    }
}

impl SystemProcess {
    fn try_wait_status(&self) -> IoResult<Option<ExitStatus>> {
        let status = recover_lock(&self.child).try_wait()?;
        if status.is_some() {
            groups::reaped(recover_lock(&self.child).id());
            recover_lock(&self.drained_stdout).extend(self.streams.drain_after_exit());
        }
        Ok(status)
    }
}

#[path = "process_node_search.rs"]
mod node_search;

#[path = "process_groups.rs"]
pub mod groups;

fn validate_public_provider(provider: &str) -> Result<(), SupervisorError> {
    matches!(provider, "claude" | "codex")
        .then_some(())
        .ok_or_else(|| SupervisorError::UnsupportedProvider(provider.into()))
}

#[cfg(unix)]
pub fn configure_process_tree(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    command.process_group(0);
}

#[cfg(not(unix))]
pub fn configure_process_tree(_: &mut Command) {}

#[cfg(unix)]
pub fn signal_process_tree(pid: i32, signal: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
    groups::signal_tree(pid, signal)
}

#[cfg(windows)]
pub fn signal_process_tree(pid: i32, signal: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
    let mut command = Command::new("taskkill");
    command.args(["/PID", &pid.to_string(), "/T"]);
    if signal == ProcessTreeSignal::Kill {
        command.arg("/F");
    }
    let status = command
        .status()
        .map_err(|error| ProcessTreeError::Failed(error.to_string()))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| ProcessTreeError::Failed(format!("taskkill exited with {status}")))
}

#[cfg(not(any(unix, windows)))]
pub fn signal_process_tree(_: i32, _: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
    Err(ProcessTreeError::Failed(
        "process-tree signaling is not implemented on this platform".into(),
    ))
}

fn recover_lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
