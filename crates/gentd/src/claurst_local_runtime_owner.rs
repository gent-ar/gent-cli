//! Effect owner for one private local llama.cpp and Claurst ACP process pair.
//!
//! This module intentionally stops at process ownership.  A later adapter will attach the ACP
//! stdin/stdout streams to `PrivateClaurstBridge`; no client or IPC code can reach this owner.

use std::{
    fs,
    io::Write,
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, TryRecvError},
    },
};

use crate::{
    claurst_acp_transport::ClaurstAcpStdio, claurst_local_runtime::LocalProcessLaunch,
    claurst_standalone_owner::ClaurstStandaloneLauncher,
};

#[path = "claurst_local_runtime_owner_process_io.rs"]
mod process_io;
use process_io::{bounded_frame, relay_acp_frames};

/// The private filesystem effect needed before `claurst acp` starts.
pub(crate) trait PrivateSettingsStore {
    fn materialize(&self, path: &Path, contents: &str) -> Result<(), String>;
}

/// Standard filesystem implementation for the daemon-owned Claurst home.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SystemPrivateSettingsStore;

impl PrivateSettingsStore for SystemPrivateSettingsStore {
    fn materialize(&self, path: &Path, contents: &str) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or_else(|| "Claurst settings path has no parent".to_owned())?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let temporary = parent.join(format!(
            ".settings-{}-{}.tmp",
            std::process::id(),
            unique_suffix()
        ));
        let result = (|| {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|error| error.to_string())?;
            file.write_all(contents.as_bytes())
                .map_err(|error| error.to_string())?;
            file.sync_all().map_err(|error| error.to_string())?;
            replace_materialized_file(&temporary, path).map_err(|error| error.to_string())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos())
}

fn replace_materialized_file(temporary: &Path, destination: &Path) -> Result<(), std::io::Error> {
    #[cfg(windows)]
    match fs::remove_file(destination) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::rename(temporary, destination)
}

/// One owned local runtime child. Implementations must make repeated shutdown safe.
pub(crate) trait LocalRuntimeProcess {
    fn exited(&mut self) -> Result<Option<String>, String>;
    fn shutdown(&mut self) -> Result<(), String>;
}

/// Starts exactly the plan-specified local process with no provider configuration inference.
pub(crate) trait LocalRuntimeLauncher {
    type Process: LocalRuntimeProcess;

    fn launch(&self, launch: &LocalProcessLaunch) -> Result<Self::Process, String>;
}

/// Process launcher used by a future private Claurst authority composition.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SystemLocalRuntimeLauncher;

pub(crate) struct SystemLocalRuntimeProcess {
    child: Child,
}

pub(crate) struct SystemClaurstAcpStdio {
    child: Child,
    stdin: ChildStdin,
    frames: Receiver<Result<Vec<u8>, String>>,
    stderr: Arc<Mutex<Vec<u8>>>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SystemClaurstStandaloneLauncher;

impl LocalRuntimeLauncher for SystemLocalRuntimeLauncher {
    type Process = SystemLocalRuntimeProcess;

    fn launch(&self, launch: &LocalProcessLaunch) -> Result<Self::Process, String> {
        let mut command = Command::new(&launch.executable);
        command
            .args(&launch.arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for key in [
            "CLAURST_BRIDGE_URL",
            "CLAURST_BRIDGE_TOKEN",
            "CLAUDE_BRIDGE_OAUTH_TOKEN",
            "CLAURST_REMOTE",
            "CLAURST_HOME",
            "USERPROFILE",
        ] {
            command.env_remove(key);
        }
        command.envs(&launch.environment);
        let child = command.spawn().map_err(|error| error.to_string())?;
        Ok(SystemLocalRuntimeProcess { child })
    }
}

impl ClaurstStandaloneLauncher for SystemClaurstStandaloneLauncher {
    type Llama = SystemLocalRuntimeProcess;
    type Acp = SystemClaurstAcpStdio;

    fn launch_llama(&self, launch: &LocalProcessLaunch) -> Result<Self::Llama, String> {
        SystemLocalRuntimeLauncher.launch(launch)
    }

    fn launch_acp(&self, launch: &LocalProcessLaunch) -> Result<Self::Acp, String> {
        let mut command = local_command(launch);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|error| error.to_string())?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Claurst ACP stdin was not piped".to_owned())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Claurst ACP stdout was not piped".to_owned())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "Claurst ACP stderr was not piped".to_owned())?;
        let (sender, frames) = mpsc::sync_channel(64);
        let stderr_capture = Arc::new(Mutex::new(Vec::new()));
        std::thread::Builder::new()
            .name("gent-claurst-acp-reader".into())
            .spawn(move || relay_acp_frames(stdout, sender))
            .map_err(|error| error.to_string())?;
        let stderr_for_reader = Arc::clone(&stderr_capture);
        std::thread::Builder::new()
            .name("gent-claurst-acp-stderr".into())
            .spawn(move || {
                let mut stderr = stderr;
                capture_acp_stderr(&mut stderr, stderr_for_reader);
            })
            .map_err(|error| error.to_string())?;
        Ok(SystemClaurstAcpStdio {
            child,
            stdin,
            frames,
            stderr: stderr_capture,
        })
    }
}

impl ClaurstAcpStdio for SystemClaurstAcpStdio {
    fn write_frame(&mut self, frame: &[u8]) -> Result<(), String> {
        self.stdin
            .write_all(frame)
            .map_err(|error| error.to_string())?;
        self.stdin.flush().map_err(|error| error.to_string())
    }

    fn try_read_frame(&mut self, maximum_bytes: usize) -> Result<Option<Vec<u8>>, String> {
        match self.frames.try_recv() {
            Ok(Ok(frame)) => bounded_frame(frame, maximum_bytes).map(Some),
            Ok(Err(error)) => Err(error),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => match self.child.try_wait() {
                Ok(Some(status)) => Err(acp_exit_error(status, &self.stderr)),
                Ok(None) => Err("Claurst ACP reader stopped unexpectedly".into()),
                Err(error) => Err(error.to_string()),
            },
        }
    }
}

#[path = "claurst_local_runtime_owner_diagnostics.rs"]
mod diagnostics;
use diagnostics::{acp_exit_error, capture_acp_stderr};

impl Drop for SystemClaurstAcpStdio {
    fn drop(&mut self) {
        let _ = self.stdin.flush();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn local_command(launch: &LocalProcessLaunch) -> Command {
    let mut command = Command::new(&launch.executable);
    command.args(&launch.arguments);
    for key in [
        "CLAURST_BRIDGE_URL",
        "CLAURST_BRIDGE_TOKEN",
        "CLAUDE_BRIDGE_OAUTH_TOKEN",
        "CLAURST_REMOTE",
        "CLAURST_HOME",
        "USERPROFILE",
    ] {
        command.env_remove(key);
    }
    command.envs(&launch.environment);
    command
}

#[cfg(test)]
mod tests {
    use super::{PrivateSettingsStore, SystemPrivateSettingsStore};

    #[test]
    fn materialize_replaces_existing_settings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let store = SystemPrivateSettingsStore;
        store.materialize(&path, "first").unwrap();
        store.materialize(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "second");
    }
}

impl LocalRuntimeProcess for SystemLocalRuntimeProcess {
    fn exited(&mut self) -> Result<Option<String>, String> {
        self.child
            .try_wait()
            .map_err(|error| error.to_string())
            .map(|status| status.map(|status| format!("local process exited with {status}")))
    }

    fn shutdown(&mut self) -> Result<(), String> {
        if self
            .child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_none()
        {
            self.child.kill().map_err(|error| error.to_string())?;
        }
        self.child
            .wait()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

#[path = "claurst_local_runtime_owner_lifecycle.rs"]
mod lifecycle;
#[path = "claurst_local_runtime_owner_readiness.rs"]
mod readiness;

#[allow(unused_imports)]
pub(crate) use lifecycle::ClaurstLocalRuntimeOwner;
pub(crate) use readiness::{HttpLlamaServerReadiness, LlamaServerReadiness};
