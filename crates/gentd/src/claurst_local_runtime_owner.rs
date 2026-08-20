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

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::VecDeque, path::PathBuf, rc::Rc};

    use super::{
        ClaurstLocalRuntimeOwner, LlamaServerReadiness, LocalRuntimeLauncher, LocalRuntimeProcess,
        PrivateSettingsStore,
    };
    use crate::{
        claurst_local_runtime::{ClaurstLocalRuntimePlan, ClaurstLocalRuntimeRequest},
        local_model_catalog::LocalModelCatalog,
    };

    #[derive(Clone, Default)]
    struct Events(Rc<RefCell<Vec<String>>>);

    struct Store(Events);
    impl PrivateSettingsStore for Store {
        fn materialize(&self, path: &std::path::Path, contents: &str) -> Result<(), String> {
            self.0
                .0
                .borrow_mut()
                .push(format!("settings:{}:{contents}", path.display()));
            Ok(())
        }
    }

    struct Process {
        name: String,
        events: Events,
    }
    impl LocalRuntimeProcess for Process {
        fn shutdown(&mut self) -> Result<(), String> {
            self.events
                .0
                .borrow_mut()
                .push(format!("stop:{}", self.name));
            Ok(())
        }
    }

    struct Launcher {
        events: Events,
        outcomes: RefCell<VecDeque<Result<String, String>>>,
    }
    impl LocalRuntimeLauncher for Launcher {
        type Process = Process;

        fn launch(
            &self,
            launch: &crate::claurst_local_runtime::LocalProcessLaunch,
        ) -> Result<Process, String> {
            let name = self.outcomes.borrow_mut().pop_front().unwrap();
            self.events
                .0
                .borrow_mut()
                .push(format!("launch:{}", launch.arguments.join(" ")));
            name.map(|name| Process {
                name,
                events: self.events.clone(),
            })
        }
    }

    struct Ready {
        events: Events,
        outcome: Result<(), String>,
    }
    impl LlamaServerReadiness for Ready {
        fn wait_ready(&self, server_url: &str) -> Result<(), String> {
            self.events
                .0
                .borrow_mut()
                .push(format!("ready:{server_url}"));
            self.outcome.clone()
        }
    }

    fn plan() -> ClaurstLocalRuntimePlan {
        let catalog = LocalModelCatalog::shipped().unwrap();
        ClaurstLocalRuntimePlan::build(
            ClaurstLocalRuntimeRequest {
                claurst_executable: PathBuf::from("/opt/gent/bin/claurst"),
                llama_server_executable: PathBuf::from("/opt/gent/bin/llama-server"),
                model_path: PathBuf::from("/opt/gent/models/model.gguf"),
                claurst_home: PathBuf::from("/opt/gent/claurst"),
                port: 18_080,
            },
            catalog.models().first().unwrap(),
        )
        .unwrap()
    }

    fn owner(
        events: Events,
        launches: Vec<Result<&str, &str>>,
        ready: Result<(), &str>,
    ) -> ClaurstLocalRuntimeOwner<Store, Launcher, Ready> {
        ClaurstLocalRuntimeOwner::new(
            Store(events.clone()),
            Launcher {
                events: events.clone(),
                outcomes: RefCell::new(
                    launches
                        .into_iter()
                        .map(|outcome| outcome.map(str::to_owned).map_err(str::to_owned))
                        .collect(),
                ),
            },
            Ready {
                events,
                outcome: ready.map_err(str::to_owned),
            },
        )
    }

    #[test]
    fn materializes_then_starts_llama_waits_and_starts_acp_before_orderly_shutdown() {
        let events = Events::default();
        let mut owner = owner(events.clone(), vec![Ok("llama"), Ok("acp")], Ok(()));
        owner.start(&plan()).unwrap();
        owner.shutdown().unwrap();
        owner.shutdown().unwrap();
        let events = events.0.borrow();
        assert!(events[0].starts_with("settings:/opt/gent/claurst/settings.json:"));
        assert_eq!(
            &events[1..],
            [
                "launch:-m /opt/gent/models/model.gguf --host 127.0.0.1 --port 18080 --jinja --ctx-size 32768 --parallel 1",
                "ready:http://127.0.0.1:18080",
                "launch:acp",
                "stop:acp",
                "stop:llama",
            ]
        );
    }

    #[test]
    fn readiness_failure_stops_llama_and_never_starts_acp() {
        let events = Events::default();
        let mut owner = owner(events.clone(), vec![Ok("llama")], Err("not yet"));
        assert!(owner.start(&plan()).unwrap_err().contains("not ready"));
        assert_eq!(
            events.0.borrow()[1..],
            [
                "launch:-m /opt/gent/models/model.gguf --host 127.0.0.1 --port 18080 --jinja --ctx-size 32768 --parallel 1",
                "ready:http://127.0.0.1:18080",
                "stop:llama",
            ]
        );
    }

    #[test]
    fn acp_launch_failure_stops_the_ready_llama_server() {
        let events = Events::default();
        let mut owner = owner(events.clone(), vec![Ok("llama"), Err("missing")], Ok(()));
        assert!(owner.start(&plan()).unwrap_err().contains("Claurst ACP"));
        assert_eq!(events.0.borrow().last(), Some(&"stop:llama".to_string()));
    }
}
