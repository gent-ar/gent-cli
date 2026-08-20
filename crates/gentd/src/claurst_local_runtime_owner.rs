//! Effect owner for one private local llama.cpp and Claurst ACP process pair.
//!
//! This module intentionally stops at process ownership.  A later adapter will attach the ACP
//! stdin/stdout streams to `PrivateClaurstBridge`; no client or IPC code can reach this owner.

use std::{
    fs,
    io::Write,
    net::{TcpStream, ToSocketAddrs},
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};

use crate::claurst_local_runtime::{ClaurstLocalRuntimePlan, LocalProcessLaunch};

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
            fs::rename(&temporary, path).map_err(|error| error.to_string())
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

/// One owned local runtime child. Implementations must make repeated shutdown safe.
pub(crate) trait LocalRuntimeProcess {
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

impl LocalRuntimeLauncher for SystemLocalRuntimeLauncher {
    type Process = SystemLocalRuntimeProcess;

    fn launch(&self, launch: &LocalProcessLaunch) -> Result<Self::Process, String> {
        let mut command = Command::new(&launch.executable);
        command
            .args(&launch.arguments)
            // ACP needs these streams for the future bridge; its process is never composed until
            // that owner is present, so no unobserved ACP process can be made reachable today.
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // A local Claurst runtime must not inherit the old remote bridge configuration.
        for key in [
            "CLAURST_BRIDGE_URL",
            "CLAURST_BRIDGE_TOKEN",
            "CLAUDE_BRIDGE_OAUTH_TOKEN",
            "CLAURST_REMOTE",
        ] {
            command.env_remove(key);
        }
        command.envs(&launch.environment);
        let child = command.spawn().map_err(|error| error.to_string())?;
        Ok(SystemLocalRuntimeProcess { child })
    }
}

impl LocalRuntimeProcess for SystemLocalRuntimeProcess {
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

/// Readiness boundary between `llama-server` launch and ACP launch.
pub(crate) trait LlamaServerReadiness {
    fn wait_ready(&self, server_url: &str) -> Result<(), String>;
}

/// Small bounded `/health` probe for the local-only llama.cpp server.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HttpLlamaServerReadiness {
    attempts: u8,
    retry_delay: Duration,
}

impl HttpLlamaServerReadiness {
    #[must_use]
    pub(crate) const fn new(attempts: u8, retry_delay: Duration) -> Self {
        Self {
            attempts,
            retry_delay,
        }
    }
}

impl Default for HttpLlamaServerReadiness {
    fn default() -> Self {
        Self::new(30, Duration::from_millis(200))
    }
}

impl LlamaServerReadiness for HttpLlamaServerReadiness {
    fn wait_ready(&self, server_url: &str) -> Result<(), String> {
        let address = server_url
            .strip_prefix("http://")
            .ok_or_else(|| "local llama.cpp server URL must use http".to_owned())?;
        if address.contains('/') {
            return Err("local llama.cpp server URL must not include a path".into());
        }
        let attempts = self.attempts.max(1);
        for attempt in 0..attempts {
            if health_check(address).is_ok() {
                return Ok(());
            }
            if attempt + 1 < attempts {
                std::thread::sleep(self.retry_delay);
            }
        }
        Err("local llama.cpp server did not become ready".into())
    }
}

fn health_check(address: &str) -> Result<(), String> {
    let socket = address
        .to_socket_addrs()
        .map_err(|error| error.to_string())?
        .next()
        .ok_or_else(|| "local llama.cpp server address is empty".to_owned())?;
    let mut stream = TcpStream::connect_timeout(&socket, Duration::from_millis(500))
        .map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .map_err(|error| error.to_string())?;
    stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .map_err(|error| error.to_string())?;
    let mut response = [0_u8; 64];
    let count =
        std::io::Read::read(&mut stream, &mut response).map_err(|error| error.to_string())?;
    let response = std::str::from_utf8(&response[..count]).map_err(|error| error.to_string())?;
    response
        .starts_with("HTTP/1.1 2")
        .then_some(())
        .ok_or_else(|| "local llama.cpp health endpoint was not successful".into())
}

struct ActiveRuntime<P> {
    llama_server: P,
    claurst_acp: P,
}

/// Owns a single local runtime and guarantees ACP is stopped before llama.cpp.
pub(crate) struct ClaurstLocalRuntimeOwner<S, L: LocalRuntimeLauncher, R> {
    settings: S,
    launcher: L,
    readiness: R,
    active: Option<ActiveRuntime<L::Process>>,
}

impl<S, L, R> ClaurstLocalRuntimeOwner<S, L, R>
where
    S: PrivateSettingsStore,
    L: LocalRuntimeLauncher,
    R: LlamaServerReadiness,
{
    #[must_use]
    pub(crate) fn new(settings: S, launcher: L, readiness: R) -> Self {
        Self {
            settings,
            launcher,
            readiness,
            active: None,
        }
    }

    /// Materializes private settings, starts llama.cpp, waits for it, then starts ACP.
    pub(crate) fn start(&mut self, plan: &ClaurstLocalRuntimePlan) -> Result<(), String> {
        if self.active.is_some() {
            return Err("a local Claurst runtime is already active".into());
        }
        self.settings
            .materialize(&plan.settings_path, &plan.settings_json)
            .map_err(|error| format!("could not materialize private Claurst settings: {error}"))?;
        let mut llama_server = self
            .launcher
            .launch(&plan.llama_server)
            .map_err(|error| format!("could not start llama.cpp server: {error}"))?;
        if let Err(error) = self.readiness.wait_ready(&plan.server_url) {
            let _ = llama_server.shutdown();
            return Err(format!("local llama.cpp server was not ready: {error}"));
        }
        let claurst_acp = match self.launcher.launch(&plan.claurst_acp) {
            Ok(process) => process,
            Err(error) => {
                let _ = llama_server.shutdown();
                return Err(format!("could not start Claurst ACP: {error}"));
            }
        };
        self.active = Some(ActiveRuntime {
            llama_server,
            claurst_acp,
        });
        Ok(())
    }

    /// Stops the dependent ACP process first, then its local model server. It is idempotent.
    pub(crate) fn shutdown(&mut self) -> Result<(), String> {
        let Some(mut active) = self.active.take() else {
            return Ok(());
        };
        let acp = active.claurst_acp.shutdown();
        let llama = active.llama_server.shutdown();
        match (acp, llama) {
            (Ok(()), Ok(())) => Ok(()),
            (acp, llama) => {
                self.active = Some(active);
                let acp_error = acp.err().unwrap_or_else(|| "ok".to_owned());
                let llama_error = llama.err().unwrap_or_else(|| "ok".to_owned());
                Err(format!(
                    "local Claurst runtime shutdown failed (acp: {}; llama.cpp: {})",
                    acp_error, llama_error
                ))
            }
        }
    }
}
