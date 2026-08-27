//! The operating-system edge for public provider processes.
//!
//! It starts only lock-validated Claude or Codex executables. Output collection is bounded at
//! the reader edge so an uncooperative provider cannot grow driver memory without limit.

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

/// A synchronous launcher for public executables with a fixed per-stream output limit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SystemLauncher {
    output_limit: usize,
    preferred_node_bin: Option<std::path::PathBuf>,
}

impl SystemLauncher {
    #[must_use]
    pub const fn new(output_limit: usize) -> Self {
        Self {
            output_limit,
            preferred_node_bin: None,
        }
    }

    /// Puts a caller-locked Node directory first for npm-installed provider shims.
    #[must_use]
    pub fn with_preferred_node(output_limit: usize, node_bin: std::path::PathBuf) -> Self {
        Self {
            output_limit,
            preferred_node_bin: Some(node_bin),
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
        if let Some(node_bin) = &self.preferred_node_bin {
            configure_locked_node_environment(&mut command, node_bin)?;
        }
        if let Some(workspace_root) = &launch.workspace_root {
            command.current_dir(workspace_root);
        }
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

/// Restricts a child to the locked Node runtime and removes inherited npm controls.
///
/// This is shared by ordinary provider shims and the private npm installer. It deliberately does
/// not inherit `PATH`: an npm shim using `env node` can resolve only the locked runtime.
///
/// # Errors
/// Returns when a safe child `PATH` cannot be constructed.
pub fn configure_locked_node_environment(
    command: &mut Command,
    node_bin: &std::path::Path,
) -> Result<(), SupervisorError> {
    let path = locked_node_path(node_bin)?;
    command.env("PATH", path);
    for variable in [
        "NODE_OPTIONS",
        "NODE_PATH",
        "npm_config_prefix",
        "NPM_CONFIG_PREFIX",
        "npm_config_userconfig",
        "NPM_CONFIG_USERCONFIG",
        "npm_config_globalconfig",
        "NPM_CONFIG_GLOBALCONFIG",
        "npm_config_registry",
        "NPM_CONFIG_REGISTRY",
        "npm_config_proxy",
        "NPM_CONFIG_PROXY",
        "npm_config_https_proxy",
        "NPM_CONFIG_HTTPS_PROXY",
    ] {
        command.env_remove(variable);
    }
    Ok(())
}

fn locked_node_path(node_bin: &std::path::Path) -> Result<std::ffi::OsString, SupervisorError> {
    #[cfg(unix)]
    let paths = [node_bin.to_path_buf(), "/usr/bin".into(), "/bin".into()];
    #[cfg(windows)]
    let paths = [node_bin.to_path_buf()];
    std::env::join_paths(paths)
        .map_err(|_| SupervisorError::Launch("locked Node path cannot form PATH".into()))
}

/// A provider-owned process group with bounded asynchronous pipe readers.
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

    /// Returns a consistent snapshot of the bounded captures collected so far.
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
            recover_lock(&self.drained_stdout).extend(self.streams.drain_after_exit());
        }
        Ok(status)
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

fn recover_lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
