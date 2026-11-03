//! Exact Linux installation resources. Construction requires the existing
//! managed-provider release authority plus its signed Linux companion.
use crate::{
    linux_provider_deployment::{FilePin, VerifiedLinuxDeployment},
    provider_executables::ManagedProviderRuntime,
    standalone_authority_release::StandaloneAuthorityRelease,
};
use gent_types::{RunVersionLock, SandboxCpuSemantics, SandboxResourceGrant};
use std::{
    collections::BTreeMap,
    fs::File,
    path::{Path, PathBuf},
};

pub(crate) struct LinuxProviderResources {
    runtime: ManagedProviderRuntime,
    release: StandaloneAuthorityRelease,
    deployment: VerifiedLinuxDeployment,
    parent: PathBuf,
    helper: RunVersionLock,
    node: RunVersionLock,
    gent: RunVersionLock,
    entries: BTreeMap<String, RunVersionLock>,
    grants: Vec<SandboxResourceGrant>,
    handles: Vec<File>,
}
fn lock(
    deployment: &VerifiedLinuxDeployment,
    pin: &FilePin,
    provider: &str,
    entry: &str,
) -> Result<(RunVersionLock, File), String> {
    use std::os::unix::fs::MetadataExt;
    let path = deployment.file_path(pin).map_err(str::to_owned)?;
    let file = File::open(&path).map_err(|_| "Linux approved resource unavailable")?;
    let before = file
        .metadata()
        .map_err(|_| "Linux approved resource metadata")?;
    let lock = gent_drivers::lock::capture(provider, &path, "signed-linux-v1", entry)
        .map_err(|_| "Linux resource identity unavailable")?;
    let held = file
        .metadata()
        .map_err(|_| "Linux retained resource metadata")?;
    let current = path.metadata().map_err(|_| "Linux resource path changed")?;
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
    if !before.is_file()
        || before.len() != pin.bytes
        || lock.digest_sha256 != pin.sha256
        || stamp(&before) != stamp(&held)
        || stamp(&before) != stamp(&current)
    {
        return Err("Linux approved resource changed".into());
    }
    Ok((lock, file))
}
impl LinuxProviderResources {
    pub(crate) fn prepare(
        runtime: ManagedProviderRuntime,
        release: StandaloneAuthorityRelease,
    ) -> Result<Self, String> {
        if runtime.executable().provider != "claude" {
            return Err("Linux contained provider contract is unavailable".into());
        }
        runtime.recheck()?;
        let deployment =
            release.linux_deployment(crate::startup::unix_seconds(), runtime.node())?;
        let parent = gent_linux_sandbox::delegation::current_service_parent("controller")
            .map_err(str::to_owned)?;
        let mut handles = Vec::new();
        let sdk = File::open(&runtime.executable().canonical_path)
            .map_err(|_| "Linux managed SDK handle unavailable")?;
        runtime.recheck()?;
        handles.push(sdk);
        let (helper, pin) = lock(
            &deployment,
            deployment.helper(),
            "gent-linux-sandbox-helper",
            "signed-linux-helper-v1",
        )?;
        handles.push(pin);
        gent_linux_sandbox::verify_helper_installation(&helper).map_err(|_|"Linux helper needs an immutable installation; user-owned AppImage helper is not admitted")?;
        let (node, pin) = lock(
            &deployment,
            deployment.node(),
            "node-mcp",
            "signed-linux-node-v1",
        )?;
        handles.push(pin);
        if runtime.node().node_path() != Path::new(&node.canonical_path)
            || runtime.node().node_digest_sha256() != node.digest_sha256
        {
            return Err("Linux MCP Node does not match managed release runtime".into());
        }
        let (gent, pin) = lock(
            &deployment,
            deployment.gent(),
            "gent-mcp",
            "signed-linux-gent-v1",
        )?;
        handles.push(pin);
        let mut entries = BTreeMap::new();
        for (name, pin) in deployment.entries() {
            let (entry, file) = lock(
                &deployment,
                pin,
                "node-mcp-entry",
                &format!("gent-node-bundle:{name}:v1"),
            )?;
            handles.push(file);
            entries.insert(name.clone(), entry);
        }
        deployment.network().map_err(str::to_owned)?;
        // The signed finite baseline policy approves MXC's explicit fixed read list;
        // no workspace, HOME, user config or arbitrary additional directory is added.
        let mut grants = gent_linux_sandbox::mxc_baseline::required_existing_roots()
            .map_err(|_| "Linux OS baseline unavailable")?
            .into_iter()
            .map(SandboxResourceGrant::RuntimeReadOnly)
            .collect::<Vec<_>>();
        // This reviewed contract is the self-contained native Claude ELF, not a
        // general npm-directory or plugin-loader grant.
        grants.push(SandboxResourceGrant::RuntimeReadOnly(PathBuf::from(
            &runtime.executable().canonical_path,
        )));
        Ok(Self {
            runtime,
            release,
            deployment,
            parent,
            helper,
            node,
            gent,
            entries,
            grants,
            handles,
        })
    }
    pub(crate) fn runtime_deployment(
        &self,
    ) -> Result<crate::provider_egress::LinuxRuntimeDeployment, String> {
        Ok(crate::provider_egress::LinuxRuntimeDeployment {
            delegated_parent: self.parent.clone(),
            helper_lock: self.helper.clone(),
            network_runtime: self.deployment.network().map_err(str::to_owned)?,
            cpu_semantics: SandboxCpuSemantics::MonitoredTotal {
                poll_ms: 5,
                wall_ms: 300_000,
            },
        })
    }
    pub(crate) fn credentials(&self) -> crate::provider_egress::LinuxCredentialDeployment {
        crate::provider_egress::LinuxCredentialDeployment {
            provider_lock: self.runtime.executable().clone(),
            helper_lock: self.helper.clone(),
            delegated_parent: self.parent.clone(),
        }
    }
    pub(crate) fn executable(&self) -> &RunVersionLock {
        self.runtime.executable()
    }
    pub(crate) fn sdk_version(&self) -> &str {
        &self.runtime.package().version
    }
    pub(crate) fn helper_lock(&self) -> &RunVersionLock {
        &self.helper
    }
    pub(crate) fn delegated_parent(&self) -> &Path {
        &self.parent
    }
    pub(crate) fn fingerprint(&self) -> &str {
        self.deployment.fingerprint()
    }
    pub(crate) fn retained_handles(&self) -> Result<Vec<File>, String> {
        self.handles
            .iter()
            .map(|f| {
                f.try_clone()
                    .map_err(|_| "Linux resource pin duplication failed".into())
            })
            .collect()
    }
    pub(crate) fn grants(&self) -> Vec<SandboxResourceGrant> {
        self.grants.clone()
    }
    #[cfg(test)]
    pub(crate) fn into_handles(self) -> Vec<File> {
        self.handles
    }
    pub(crate) fn mcp_for_launch(
        &self,
        launch: &gent_drivers::supervisor::ProviderLaunch,
    ) -> Result<Option<crate::provider_egress::LinuxMcpDeployment>, String> {
        let indices = launch
            .arguments
            .iter()
            .enumerate()
            .filter(|(_, v)| *v == "--mcp-config")
            .map(|(i, _)| i + 1)
            .collect::<Vec<_>>();
        if indices.is_empty() {
            return Ok(None);
        }
        if indices.len() != 1 {
            return Err("Linux MCP configuration is ambiguous".into());
        }
        let source = launch
            .arguments
            .get(indices[0])
            .ok_or("Linux MCP configuration is absent")?;
        let config = crate::contained_claude_mcp::HostMcpConfig::capture(Path::new(source))
            .map_err(|_| "Linux MCP selected set unavailable")?;
        let mut programs = BTreeMap::new();
        let mut node_entries = BTreeMap::new();
        for (name, server) in config.servers() {
            if server.command == self.gent.canonical_path {
                programs.insert(name.clone(), self.gent.clone());
            } else if server.command == self.node.canonical_path {
                let entry = self
                    .entries
                    .get(name)
                    .ok_or("Linux selected Node server is not approved")?;
                if server.args != [entry.canonical_path.clone()] {
                    return Err("Linux selected Node entry does not match release".into());
                }
                programs.insert(name.clone(), self.node.clone());
                node_entries.insert(name.clone(), entry.clone());
            } else {
                return Err("Linux selected MCP executable is not approved".into());
            }
        }
        Ok(Some(crate::provider_egress::LinuxMcpDeployment {
            delegated_parent: self.parent.clone(),
            helper: self.helper.clone(),
            programs,
            node_entries,
        }))
    }
}
impl crate::permission_backend_registry::VerifiedProviderFactoryResources
    for LinuxProviderResources
{
    fn recheck(&self) -> Result<(), String> {
        self.runtime.recheck()?;
        let current = self
            .release
            .linux_deployment(crate::startup::unix_seconds(), self.runtime.node())?;
        if current.fingerprint() != self.deployment.fingerprint() {
            return Err("Linux deployment generation changed".into());
        }
        if gent_linux_sandbox::delegation::current_service_parent("controller")
            .map_err(str::to_owned)?
            != self.parent
        {
            return Err("Linux delegated service ownership changed".into());
        }
        Ok(())
    }
}
