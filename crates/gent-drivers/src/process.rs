use std::io::{Read, Result as IoResult};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal};
use crate::lock::rechecked_identity;
use crate::process_exit::ProcessExit;
use crate::process_streams::ProcessStreams;
pub use crate::process_streams::{CapturedStream, ProcessOutput};
use crate::provider_input::ProviderInput;
use crate::supervisor::{ProcessLauncher, ProviderLaunch, ProviderProcess, SupervisorError};
use node_search::NodeSearch;
pub use node_search::configure_locked_node_environment;

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
        let mut command = gent_types::hidden_command(&launch.executable);
        command
            .args(&launch.arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        self.node.configure(&mut command)?;
        node_search::provider_environment(&mut command, &launch.provider);
        if let Some(workspace_root) = &launch.workspace_root {
            command.current_dir(workspace_root);
        }
        configure_process_tree(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| SupervisorError::Launch(error.to_string()))?;
        groups::spawned(&child, &launch.executable);
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
        SystemProcess::new(child, stdin, stdout, stderr, self.output_limit)
            .map_err(|error| SupervisorError::Launch(error.to_string()))
    }
}

#[derive(Debug)]
pub struct SystemProcess {
    pid: u32,
    exit: Arc<ProcessExit>,
    input: ProviderInput,
    streams: ProcessStreams,
    tree: Mutex<Option<OwnedTree>>,
}

#[cfg(windows)]
type OwnedTree = win32job::Job;
#[cfg(not(windows))]
type OwnedTree = u32;

impl SystemProcess {
    pub(crate) fn new(
        mut child: Child,
        stdin: ChildStdin,
        stdout: impl Read + Send + 'static,
        stderr: impl Read + Send + 'static,
        output_limit: usize,
    ) -> Result<Self, ProcessTreeError> {
        #[cfg(windows)]
        let tree = windows::own_tree(&child);
        #[cfg(not(windows))]
        let tree: Result<OwnedTree, ProcessTreeError> = Ok(child.id());
        let owned = tree.and_then(|tree| {
            ProviderInput::spawn(stdin)
                .map(|input| (tree, input))
                .map_err(|error| ProcessTreeError::Failed(error.to_string()))
        });
        let (tree, input) = match owned {
            Ok(owned) => owned,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let pid = child.id();
        let exit = ProcessExit::watch(child).map_err(|error| {
            let _ = i32::try_from(pid).map(|pid| signal_process_tree(pid, ProcessTreeSignal::Kill));
            ProcessTreeError::Failed(error.to_string())
        })?;
        Ok(Self {
            pid,
            exit,
            input,
            streams: ProcessStreams::new(stdout, stderr, output_limit),
            tree: Mutex::new(Some(tree)),
        })
    }

    fn release_tree(&self) {
        let Some(tree) = recover_lock(&self.tree).take() else {
            return;
        };
        #[cfg(windows)]
        drop(tree);
        #[cfg(not(windows))]
        if let Ok(group) = i32::try_from(tree) {
            let _ = signal_process_tree(group, ProcessTreeSignal::Kill);
        }
    }

    #[cfg(windows)]
    pub(crate) fn peek_owned_exit_status(
        &self,
    ) -> Result<Option<std::process::ExitStatus>, ProcessTreeError> {
        self.exit.peek()
    }

    #[cfg(windows)]
    pub(crate) fn kill_owned(&self) -> Result<(), ProcessTreeError> {
        let status = self.exit.lock();
        if status.is_some() {
            return Ok(());
        }
        windows::terminate(self.pid)
    }

    #[cfg(windows)]
    pub(crate) fn reap_owned(&self) -> bool {
        self.exit.wait_for(std::time::Duration::from_secs(3))
    }

    pub fn wait(&self) -> IoResult<ExitStatus> {
        self.streams.join_after_tree_exit();
        let status = self.exit.wait().map_err(std::io::Error::other)?;
        self.settle_exit();
        Ok(status)
    }

    #[must_use]
    pub fn output(&self) -> ProcessOutput {
        self.streams.output()
    }
}

impl ProcessTreeControl for SystemProcess {
    fn signal_tree(&self, signal: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        if cfg!(windows) && matches!(self.exit.peek(), Ok(Some(_))) {
            return Ok(());
        }
        let pid = self
            .pid
            .try_into()
            .map_err(|_| ProcessTreeError::Failed("provider process id overflowed".into()))?;
        signal_process_tree(pid, signal)
    }
}

impl ProviderProcess for SystemProcess {
    fn write_frame(&self, frame: &[u8]) -> Result<(), ProcessTreeError> {
        self.input.send(frame)
    }

    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
        self.input.close();
        Ok(())
    }

    fn next_stdout_chunk(&self) -> Result<Option<Vec<u8>>, ProcessTreeError> {
        Ok(self.streams.next_stdout_chunk())
    }

    fn try_exit_code(&self) -> Result<Option<Option<i32>>, ProcessTreeError> {
        self.try_wait_status()
            .map(|status| status.map(|status| status.code()))
            .map_err(|error| ProcessTreeError::Failed(error.to_string()))
    }
}

impl SystemProcess {
    fn try_wait_status(&self) -> IoResult<Option<ExitStatus>> {
        let status = self.exit.peek().map_err(std::io::Error::other)?;
        if status.is_some() {
            self.settle_exit();
        }
        Ok(status)
    }

    fn settle_exit(&self) {
        self.release_tree();
        groups::reaped(self.pid);
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
    windows::signal_tree(pid, signal)
}

#[cfg(windows)]
#[path = "process_windows.rs"]
mod windows;

#[cfg(not(any(unix, windows)))]
pub fn signal_process_tree(_: i32, _: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
    Err(ProcessTreeError::Failed(
        "process-tree signaling is not implemented on this platform".into(),
    ))
}

fn recover_lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(all(test, windows))]
#[path = "process_sandbox_pipe_lifetime_tests.rs"]
mod sandbox_pipe_lifetime_tests;
