//! Native host MCP composition. Full-set admission precedes every spawn.
//! Installation locks are supplied by the runtime owner, never discovered on PATH.
use super::{
    mcp::{Bridge, Error, Route},
    mcp_authorizer::EvidenceGate,
    mcp_deferred::DeferredPeer,
    mcp_journal::DurableJournal,
    mcp_stdio::Spec,
};
use crate::contained_claude_mcp::HostMcpConfig;
use gent_types::RunVersionLock;
use std::{
    collections::BTreeMap,
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

pub(crate) struct LinuxMcpDeployment {
    pub delegated_parent: PathBuf,
    pub helper: RunVersionLock,
    /// Exact selected server names, including all unsupported profiles (rejected).
    pub programs: BTreeMap<String, RunVersionLock>,
    pub node_entries: BTreeMap<String, RunVersionLock>,
}
pub struct McpOwner {
    pub gate: Arc<EvidenceGate>,
    pub bridge: Arc<Bridge>,
    pub worker_path: PathBuf,
    _worker: File,
    _host_root: Arc<tempfile::TempDir>,
    peers: Vec<Arc<DeferredPeer>>,
}
fn overlap(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}
fn checked(lock: &RunVersionLock) -> Result<(), Error> {
    if gent_drivers::lock::rechecked_identity(lock).map_err(|_| Error::Unprovable)? != *lock {
        return Err(Error::Unprovable);
    }
    Ok(())
}
// Retain the exact opened generation, not merely an arbitrary File pin.
fn retained_file_with(
    path: &Path,
    min_bytes: u64,
    max_bytes: u64,
    verify: impl FnOnce() -> Result<(), Error>,
) -> Result<File, Error> {
    use std::os::unix::fs::MetadataExt;
    if path.canonicalize().map_err(|_| Error::Denied)? != path {
        return Err(Error::Denied);
    }
    let file = File::open(path).map_err(|_| Error::Denied)?;
    let before = file.metadata().map_err(|_| Error::Denied)?;
    if !before.is_file() || before.len() < min_bytes || before.len() > max_bytes {
        return Err(Error::Denied);
    }
    verify()?;
    let held = file.metadata().map_err(|_| Error::Denied)?;
    let current = std::fs::metadata(path).map_err(|_| Error::Denied)?;
    let stamp = |m: &std::fs::Metadata| {
        (
            m.dev(),
            m.ino(),
            m.len(),
            m.mtime(),
            m.mtime_nsec(),
            m.ctime(),
            m.ctime_nsec(),
        )
    };
    if stamp(&before) != stamp(&held) || stamp(&before) != stamp(&current) {
        return Err(Error::Denied);
    }
    Ok(file)
}
fn retained_image(lock: &RunVersionLock, max_bytes: u64) -> Result<File, Error> {
    retained_file_with(Path::new(&lock.canonical_path), 64, max_bytes, || {
        checked(lock)
    })
}
impl McpOwner {
    pub fn prepare(
        fence: Arc<super::run_fence::RunFence>,
        token: &str,
        original: &Path,
        worker_root: &Path,
        control: &crate::ordinary_lifecycle_control::OrdinaryLifecycleControl,
        deployment: &LinuxMcpDeployment,
    ) -> Result<Self, Error> {
        use std::os::unix::fs::PermissionsExt;
        let config = HostMcpConfig::capture(original).map_err(|_| Error::Protocol)?;
        super::mcp_linux_roster::validate(&config, &deployment.programs, &deployment.node_entries)?;
        if !fence.current()
            || config.servers().keys().ne(deployment.programs.keys())
            || gent_linux_sandbox::delegation::current_service_parent("controller")
                .map_err(|_| Error::Unprovable)?
                != deployment.delegated_parent
        {
            return Err(Error::Denied);
        }
        checked(&deployment.helper)?;
        let workspace = Path::new(&fence.workspace.canonical_path)
            .canonicalize()
            .map_err(|_| Error::Denied)?;
        let worker_root = worker_root.canonicalize().map_err(|_| Error::Denied)?;
        // Explicit mode: tempfile's default respects umask and may produce 0755.
        let host_root = Arc::new(
            tempfile::Builder::new()
                .prefix("gent-host-mcp-")
                .permissions(std::fs::Permissions::from_mode(0o700))
                .tempdir()
                .map_err(|_| Error::Protocol)?,
        );
        let cwd = host_root.path().canonicalize().map_err(|_| Error::Denied)?;
        if overlap(&cwd, &workspace) || overlap(&cwd, &worker_root) {
            return Err(Error::Denied);
        }
        let mut admitted = BTreeMap::new();
        // No peer/process exists during this loop. One unsupported member rejects ALL.
        for (name, server) in config.servers() {
            let lock = deployment.programs.get(name).ok_or(Error::Denied)?;
            checked(lock)?;
            let program = Path::new(&server.command)
                .canonicalize()
                .map_err(|_| Error::Denied)?;
            let admission = if lock.provider == "node-mcp" {
                admit_node(
                    name,
                    server,
                    lock,
                    program,
                    deployment,
                    &workspace,
                    &worker_root,
                )?
            } else {
                admit_gent(
                    server,
                    lock,
                    program,
                    &workspace,
                    &worker_root,
                    &cwd,
                    &host_root,
                )?
            };
            admitted.insert(name.clone(), admission);
        }
        let gate = EvidenceGate::new(fence.clone());
        let source = config.source_sha256().to_owned();
        let generation = uuid::Uuid::new_v4().simple().to_string();
        let scope = gate.scope(&source, &generation).ok_or(Error::Revoked)?;
        let scope_gate = gate.clone();
        let journal = Arc::new(DurableJournal::new(
            Arc::new(fence.ledger.clone()),
            Arc::new(move || scope_gate.scope(&source, &generation)),
        ));
        let startup = super::mcp_operations::acquire(control)?;
        let mut routes = BTreeMap::new();
        let mut paths = BTreeMap::new();
        let mut peers = Vec::new();
        for (name, server) in config.servers() {
            let id = uuid::Uuid::new_v4().simple().to_string();
            let (program, pins, args, env, peer_cwd, peer_authority) =
                admitted.remove(name).ok_or(Error::Denied)?;
            let launch_gate = gate.clone();
            let launch_scope = scope.clone();
            let native = gent_linux_sandbox::host_stdio::Config {
                delegation_parent: deployment.delegated_parent.clone(),
                helper: deployment.helper.clone(),
                program: deployment.programs[name].clone(),
                cwd: peer_cwd.clone(),
                args: args.clone(),
                env: env.clone(),
                lifetime: Duration::from_secs(60),
            };
            let peer = DeferredPeer::new_owned_linux(
                Spec {
                    launch_guard: Arc::new(move || launch_gate.admitted(&launch_scope)),
                    authority: peer_authority,
                    startup: Some(startup.clone()),
                    program,
                    args,
                    env: env.clone(),
                    cwd: peer_cwd.clone(),
                    pins,
                    secrets: super::mcp_secrets::captured(&server.env, &server.args),
                    timeout: Duration::from_secs(30),
                },
                native,
            )?;
            routes.insert(id.clone(), Route::new(name.clone(), peer.clone())?);
            paths.insert(name.clone(), id);
            peers.push(peer);
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
        let worker = config
            .materialize(&worker_root, &paths)
            .map_err(|_| Error::Protocol)?;
        super::mcp_linux_roster::validate(&config, &deployment.programs, &deployment.node_entries)?;
        if !fence.current() {
            return Err(Error::Revoked);
        }
        Ok(Self {
            gate,
            bridge,
            worker_path: worker.path,
            _worker: worker.handle,
            _host_root: host_root,
            peers,
        })
    }
    pub fn activate(&self) -> Result<(), Error> {
        for peer in &self.peers {
            peer.activate()?;
        }
        Ok(())
    }
}

type Admission = (
    PathBuf,
    Vec<File>,
    Vec<String>,
    BTreeMap<String, String>,
    PathBuf,
    Arc<tempfile::TempDir>,
);

fn admit_node(
    name: &str,
    server: &crate::contained_claude_mcp::HostStdioServer,
    lock: &RunVersionLock,
    program: PathBuf,
    deployment: &LinuxMcpDeployment,
    workspace: &Path,
    worker_root: &Path,
) -> Result<Admission, Error> {
    use std::os::unix::fs::PermissionsExt;
    let role = gent_linux_sandbox::node_profile::Role::from_server(name).ok_or(Error::Denied)?;
    let entry = deployment.node_entries.get(name).ok_or(Error::Denied)?;
    checked(entry)?;
    if program != Path::new(&lock.canonical_path)
        || overlap(&program, workspace)
        || overlap(&program, worker_root)
    {
        return Err(Error::Denied);
    }
    let entry_path = Path::new(&entry.canonical_path);
    if overlap(entry_path, workspace) || overlap(entry_path, worker_root) {
        return Err(Error::Denied);
    }
    // Independent roots: dropping the owner's common root cannot
    // recursively remove a live/Unknown peer's retained authority.
    let authority = Arc::new(
        tempfile::Builder::new()
            .prefix("gent-node-mcp-")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .map_err(|_| Error::Denied)?,
    );
    let private = authority.path().canonicalize().map_err(|_| Error::Denied)?;
    if overlap(&private, workspace) || overlap(&private, worker_root) {
        return Err(Error::Denied);
    }
    let env = gent_linux_sandbox::node_profile::selected_environment(
        role,
        &server.env,
        private.to_str().ok_or(Error::Denied)?,
    )
    .ok_or(Error::Denied)?;
    let args = vec![
        name.to_owned(),
        serde_json::to_string(entry).map_err(|_| Error::Protocol)?,
    ];
    let image = retained_image(lock, 512 * 1024 * 1024)?;
    let entry_pin = retained_image(entry, 2 * 1024 * 1024)?;
    Ok((
        program,
        vec![image, entry_pin],
        args,
        env,
        private,
        authority,
    ))
}

fn admit_gent(
    server: &crate::contained_claude_mcp::HostStdioServer,
    lock: &RunVersionLock,
    program: PathBuf,
    workspace: &Path,
    worker_root: &Path,
    cwd: &Path,
    host_root: &Arc<tempfile::TempDir>,
) -> Result<Admission, Error> {
    use std::os::unix::fs::MetadataExt;
    if lock.provider != "gent-mcp"
        || program != Path::new(&lock.canonical_path)
        || overlap(&program, workspace)
        || overlap(&program, worker_root)
        || !gent_linux_sandbox::host_stdio::validate_profile(&program, &server.args, &server.env)
    {
        return Err(Error::Denied);
    }
    let data = Path::new(&server.args[2])
        .canonicalize()
        .map_err(|_| Error::Denied)?;
    if overlap(&data, workspace) || data.starts_with(worker_root) || !data.is_dir() {
        return Err(Error::Denied);
    }
    // The core seals/rechecks the exact program lock before private Start.
    let image = File::open(&program).map_err(|_| Error::Denied)?;
    let image_meta = image.metadata().map_err(|_| Error::Denied)?;
    checked(lock)?;
    let path_meta = std::fs::metadata(&program).map_err(|_| Error::Denied)?;
    if (image_meta.dev(), image_meta.ino()) != (path_meta.dev(), path_meta.ino()) {
        return Err(Error::Denied);
    }
    let (data_pin, data_cap) =
        super::mcp_linux_paths::pin_data(Path::new(&server.args[2]), workspace, worker_root)?;
    let mut args = server.args.clone();
    args[2] = data_cap.to_str().ok_or(Error::Denied)?.into();
    Ok((
        program,
        vec![image, data_pin],
        args,
        server.env.clone(),
        cwd.to_path_buf(),
        host_root.clone(),
    ))
}

#[cfg(test)]
mod retained_pin_tests {
    use super::*;
    #[test]
    fn exact_retained_generation_survives_path_rename() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("image");
        std::fs::write(&path, b"approved-generation").unwrap();
        let file = retained_file_with(&path, 1, 1024, || Ok(())).unwrap();
        std::fs::rename(&path, root.path().join("old")).unwrap();
        std::fs::write(&path, b"replacement").unwrap();
        use std::io::Read;
        let mut bytes = Vec::new();
        (&file).read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"approved-generation");
    }
    #[test]
    fn replacement_during_verification_rejected() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("image");
        std::fs::write(&path, b"approved").unwrap();
        assert!(
            retained_file_with(&path, 1, 1024, || {
                std::fs::rename(&path, root.path().join("old")).unwrap();
                std::fs::write(&path, b"replaced").unwrap();
                Ok(())
            })
            .is_err()
        );
    }
    #[test]
    fn same_inode_mutation_during_verification_rejected() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("image");
        std::fs::write(&path, b"approved").unwrap();
        assert!(
            retained_file_with(&path, 1, 1024, || {
                std::fs::write(&path, b"changed-size").unwrap();
                Ok(())
            })
            .is_err()
        );
    }
    #[test]
    fn entry_bounds_reject_before_hashing() {
        for len in [63u64, 2 * 1024 * 1024 + 1] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("entry");
            File::create(&path).unwrap().set_len(len).unwrap();
            let called = std::cell::Cell::new(false);
            assert!(
                retained_file_with(&path, 64, 2 * 1024 * 1024, || {
                    called.set(true);
                    Ok(())
                })
                .is_err()
            );
            assert!(!called.get());
        }
    }
}
