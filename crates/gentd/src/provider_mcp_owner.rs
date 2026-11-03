use super::mcp_paths::{host_data, overlap, pin};
// Host-only stdio cohort, secret-free worker config and retained run authority.
use super::{
    mcp::{Bridge, Error, Route},
    mcp_authorizer::EvidenceGate,
    mcp_journal::DurableJournal,
    mcp_stdio::Spec,
};
use crate::contained_claude_mcp::HostMcpConfig;
use std::{collections::BTreeMap, fs::File, path::Path, sync::Arc, time::Duration};

pub struct McpOwner {
    pub gate: Arc<EvidenceGate>,
    pub bridge: Arc<Bridge>,
    pub worker_path: std::path::PathBuf,
    _worker: File,
    _host_root: Arc<tempfile::TempDir>,
    peers: Vec<Arc<super::mcp_deferred::DeferredPeer>>,
    claurst_clients: Option<(String, serde_json::Value)>,
}
impl McpOwner {
    pub fn prepare(
        fence: Arc<super::run_fence::RunFence>,
        token: &str,
        original: &Path,
        worker_root: &Path,
        control: &crate::ordinary_lifecycle_control::OrdinaryLifecycleControl,
    ) -> Result<Self, Error> {
        let config = HostMcpConfig::capture(original).map_err(|_| Error::Protocol)?;
        Self::prepare_config(fence, token, config, worker_root, control, false)
    }
    pub fn prepare_claurst(
        fence: Arc<super::run_fence::RunFence>,
        token: &str,
        original: &Path,
        expected_settings_sha256: &str,
        worker_root: &Path,
        control: &crate::ordinary_lifecycle_control::OrdinaryLifecycleControl,
    ) -> Result<Self, Error> {
        let config = HostMcpConfig::capture_claurst(original).map_err(|_| Error::Protocol)?;
        if config.source_sha256() != expected_settings_sha256 {
            return Err(Error::Denied);
        }
        Self::prepare_config(fence, token, config, worker_root, control, true)
    }
    fn prepare_config(
        fence: Arc<super::run_fence::RunFence>,
        token: &str,
        config: HostMcpConfig,
        worker_root: &Path,
        control: &crate::ordinary_lifecycle_control::OrdinaryLifecycleControl,
        claurst: bool,
    ) -> Result<Self, Error> {
        // Preflight the whole set before starting even the first host process.
        for server in config.servers().values() {
            super::mcp_profiles::validate(Path::new(&server.command), &server.args, &server.env)?;
        }
        let host_root = Arc::new(
            tempfile::Builder::new()
                .prefix("gent-host-mcp-")
                .tempdir()
                .map_err(|_| Error::Protocol)?,
        );
        let canonical = host_root
            .path()
            .canonicalize()
            .map_err(|_| Error::Protocol)?;
        let workspace = Path::new(&fence.workspace.canonical_path)
            .canonicalize()
            .map_err(|_| Error::Protocol)?;
        if overlap(&canonical, &workspace) || overlap(&canonical, worker_root) {
            return Err(Error::Denied);
        }
        let gate = EvidenceGate::new(fence.clone());
        let source = config.source_sha256().to_owned();
        let generation = uuid::Uuid::new_v4().simple().to_string();
        let admitted_scope = gate.scope(&source, &generation).ok_or(Error::Revoked)?;
        let scope_gate = gate.clone();
        let journal = Arc::new(DurableJournal::new(
            Arc::new(fence.ledger.clone()),
            Arc::new(move || scope_gate.scope(&source, &generation)),
        ));
        let mut routes = BTreeMap::new();
        let mut paths = BTreeMap::new();
        let mut peers = Vec::new();
        let startup = super::mcp_operations::acquire(control)?;
        for (name, server) in config.servers() {
            let id = uuid::Uuid::new_v4().simple().to_string();
            let (program, mut pins) = pin(Path::new(&server.command), &workspace, worker_root)?;
            let mut env = BTreeMap::new();
            for key in [
                "SystemRoot",
                "ComSpec",
                "PATHEXT",
                "PATH",
                "USERPROFILE",
                "HOME",
                "APPDATA",
                "LOCALAPPDATA",
                "PROGRAMFILES",
                "PROGRAMFILES(X86)",
            ] {
                if let Ok(value) = std::env::var(key) {
                    env.insert(key.to_owned(), value);
                }
            }
            env.extend(server.env.clone());
            let secrets = super::mcp_secrets::captured(&server.env, &server.args);
            let mut args = server.args.clone();
            if matches!(
                super::mcp_profiles::validate(
                    Path::new(&server.command),
                    &server.args,
                    &server.env
                )?,
                super::mcp_profiles::Profile::Node
            ) {
                let (entry, files) = pin(Path::new(&args[0]), &workspace, worker_root)?;
                let argument = super::mcp_paths::ordinary_argument(&entry)?;
                args[0] = argument.to_str().ok_or(Error::Protocol)?.into();
                pins.extend(files);
            } else {
                let (data, files) = host_data(Path::new(&args[1]), &workspace, worker_root)?;
                args[1] = data.to_str().ok_or(Error::Protocol)?.into();
                pins.extend(files);
            }
            let before_spawn = gate.clone();
            let launch_scope = admitted_scope.clone();
            let peer = super::mcp_deferred::DeferredPeer::new(Spec {
                launch_guard: Arc::new(move || before_spawn.admitted(&launch_scope)),
                authority: host_root.clone(),
                startup: Some(startup.clone()),
                program,
                args,
                env,
                cwd: canonical.clone(),
                pins,
                secrets,
                timeout: Duration::from_secs(120),
            });
            routes.insert(id.clone(), Route::new(name.clone(), peer.clone())?);
            peers.push(peer);
            paths.insert(name.clone(), id);
        }
        let current = fence.clone();
        let control = control.clone();
        let bridge = Arc::new(
            Bridge::new(
                token.into(),
                routes,
                Arc::new(move || current.current()),
                gate.clone(),
                journal,
            )?
            .with_operations(Arc::new(move || super::mcp_operations::acquire(&control))),
        );
        let worker = if claurst {
            config.materialize_claurst(worker_root, &paths)
        } else {
            config.materialize(worker_root, &paths)
        }
        .map_err(|_| Error::Protocol)?;
        let claurst_clients = if claurst {
            use std::io::Read;
            let mut bytes = Vec::new();
            let file = worker.handle.try_clone().map_err(|_| Error::Protocol)?;
            file.take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| Error::Protocol)?;
            if bytes.len() > 1024 * 1024 {
                return Err(Error::Protocol);
            }
            let value: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|_| Error::Protocol)?;
            let servers = value
                .get("mcpServers")
                .filter(|v| v.is_array())
                .ok_or(Error::Protocol)?
                .clone();
            Some((config.source_sha256().into(), servers))
        } else {
            None
        };
        // Dropping config releases only the original source read lock; its exact
        // approved snapshots are now retained by each host process actor.
        Ok(Self {
            gate,
            bridge,
            worker_path: worker.path,
            _worker: worker.handle,
            _host_root: host_root,
            peers,
            claurst_clients,
        })
    }
    pub(super) fn claurst_servers(
        &self,
        source_sha256: &str,
    ) -> Result<Option<serde_json::Value>, Error> {
        match &self.claurst_clients {
            Some((expected, clients)) if expected == source_sha256 => Ok(Some(clients.clone())),
            Some(_) => Err(Error::Denied),
            None => Ok(None),
        }
    }
    pub fn activate(&self) -> Result<(), Error> {
        for peer in &self.peers {
            peer.activate()?;
        }
        Ok(())
    }
}
