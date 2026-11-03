use std::{
    collections::BTreeSet,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use gent_types::{
    AgentChatProvider, ContainedProviderRoute, RunVersionLock, SandboxResourceGrant,
    SandboxShellRuntime,
};
use sha2::{Digest, Sha256};

use crate::{
    provider_executables::ManagedProviderRuntime, windows_sandbox_runtime::PinnedSandboxHelper,
};

#[path = "windows_command_closure.rs"]
pub(crate) mod command_closure;
#[path = "windows_provider_manifest.rs"]
pub(crate) mod manifest;
use manifest::{ProviderResources, ResourceRoot};

#[path = "windows_globalization_resources.rs"]
mod globalization;

pub(crate) struct ManagedProviderResources {
    provider: AgentChatProvider,
    runtime: ManagedProviderRuntime,
    contract: ProviderResources,
    grants: Vec<SandboxResourceGrant>,
    shell: SandboxShellRuntime,
    handles: Vec<File>,
}

impl ManagedProviderResources {
    pub(crate) fn grants(&self) -> Vec<SandboxResourceGrant> {
        self.grants.clone()
    }
    pub(crate) fn shell(&self) -> SandboxShellRuntime {
        self.shell.clone()
    }
    pub(crate) fn into_handles(self) -> Vec<File> {
        self.handles
    }
    pub(crate) fn permits_claude_credentials(&self, lock: &RunVersionLock) -> bool {
        self.contract.credential_contract == "claude-loopback-v1"
            && self.runtime.executable() == lock
    }
    pub(crate) fn permits_codex_credentials(&self, lock: &RunVersionLock) -> bool {
        self.contract.credential_contract == "codex-file-responses-v1"
            && self.runtime.executable() == lock
    }
}

impl crate::permission_backend_registry::VerifiedProviderFactoryResources
    for ManagedProviderResources
{
    fn provider(&self) -> AgentChatProvider {
        self.provider
    }
    fn executable_lock(&self) -> &RunVersionLock {
        self.runtime.executable()
    }
    fn command_egress(&self) -> Option<gent_types::SandboxCommandEgress> {
        self.shell.command_egress()
    }
    fn recheck(&self) -> Result<(), String> {
        self.runtime.recheck()
    }
}

pub(crate) fn managed(
    helper: &PinnedSandboxHelper,
    runtime: ManagedProviderRuntime,
) -> Result<ManagedProviderResources, String> {
    let provider = AgentChatProvider::from_contained_id(&runtime.executable().provider)
        .filter(|provider| {
            provider.contained_contract().route == ContainedProviderRoute::ManagedPackage
        })
        .ok_or("managed public provider runtime required")?;
    runtime.recheck()?;
    let root = helper.runtime_root()?;
    let (contract, manifest_handle) = manifest::load(root, &runtime.executable().provider)?;
    contract.require_managed(&runtime)?;
    command_closure::require_command_closure(&contract)?;
    let node_root = runtime
        .node()
        .node_path()
        .parent()
        .and_then(Path::parent)
        .ok_or("managed Node runtime layout is invalid")?;
    let git_root = root.join("git");
    let powershell_root = root.join("powershell");
    let powershell_launcher_root = root.join("powershell-launcher");
    let os = crate::windows_os_resources::verify_os_libraries(helper, &contract.os_libraries)?;
    let windows_root = os
        .system_directory()
        .parent()
        .ok_or("verified Windows directory unavailable")?;
    let mut grants = Vec::new();
    let mut handles = vec![manifest_handle];
    let mut paths = BTreeSet::new();
    for resource in &contract.files {
        let directory = match resource.root {
            ResourceRoot::NodeRuntime => node_root,
            ResourceRoot::Package => runtime.package_root(),
            ResourceRoot::GitRuntime => &git_root,
            ResourceRoot::PowerShellRuntime => &powershell_root,
            ResourceRoot::PowerShellLauncher => &powershell_launcher_root,
            ResourceRoot::OsGlobalizationData => windows_root,
        };
        let path = manifest::resource_path(directory, &resource.path)?;
        pin(&path, &resource.sha256, &mut grants, &mut handles)?;
        if resource.root == ResourceRoot::OsGlobalizationData {
            globalization::verify(helper, &path, &resource.sha256)?;
        }
        paths.insert(path);
    }
    for required in [
        runtime.node().node_path(),
        runtime.node().npm_path(),
        runtime.node().npm_cli_path(),
    ] {
        if !paths.contains(required) {
            return Err("managed Node/npm closure is incomplete".into());
        }
    }
    let image = PathBuf::from(&runtime.executable().canonical_path);
    pin(
        &image,
        &runtime.executable().digest_sha256,
        &mut grants,
        &mut handles,
    )?;
    pin(
        Path::new(&helper.image_lock().canonical_path),
        &helper.image_lock().digest_sha256,
        &mut grants,
        &mut handles,
    )?;
    manifest::require_git_mounts(&contract)?;
    let shell_path = manifest::resource_path(&git_root, "usr/bin/bash.exe")?;
    if !paths.contains(&shell_path) {
        return Err("managed Git shell closure is incomplete".into());
    }
    let shell_lock =
        gent_drivers::lock::capture("git-bash", &shell_path, "5.2.15", "managed-git-bash-v1")
            .map_err(|_| "managed Git shell unavailable")?;
    if Some(shell_lock.digest_sha256.as_str()) != option_env!("GENT_WINDOWS_GIT_BASH_SHA256") {
        return Err("managed Git shell does not match its reviewed build pin".into());
    }
    grants.extend(
        os.paths()
            .into_iter()
            .map(SandboxResourceGrant::RuntimeReadOnly),
    );
    handles.extend(os.into_handles());
    let mut shell =
        SandboxShellRuntime::new(shell_lock).map_err(|_| "managed Git shell contract rejected")?;
    if !contract
        .files
        .iter()
        .any(|f| f.root == ResourceRoot::OsGlobalizationData)
    {
        return Err("managed provider globalization data closure is incomplete".into());
    }
    for required in [
        "pwsh.exe",
        "pwsh.dll",
        "pwsh.deps.json",
        "pwsh.runtimeconfig.json",
        "hostfxr.dll",
        "hostpolicy.dll",
        "coreclr.dll",
        "System.Private.CoreLib.dll",
        "System.Management.Automation.dll",
    ] {
        if !paths.contains(&manifest::resource_path(&powershell_root, required)?) {
            return Err("managed provider PowerShell runtime closure is incomplete".into());
        }
    }
    let path = manifest::resource_path(&powershell_root, "pwsh.exe")?;
    let lock =
        gent_drivers::lock::capture("powershell7", &path, "7-portable", "managed-powershell-v1")
            .map_err(|_| "managed PowerShell runtime unavailable")?;
    shell = shell
        .with_powershell(lock)
        .map_err(|_| "managed PowerShell contract rejected")?;
    let launcher = manifest::resource_path(&powershell_launcher_root, "pwsh.exe")?;
    if !paths.contains(&launcher) {
        return Err("managed PowerShell launcher closure is incomplete".into());
    }
    let lock = gent_drivers::lock::capture(
        "powershell7-launcher",
        &launcher,
        "1",
        "managed-powershell-launcher-v1",
    )
    .map_err(|_| "managed PowerShell launcher unavailable")?;
    shell = shell
        .with_powershell_launcher(lock)
        .and_then(SandboxShellRuntime::with_brokered_command_egress)
        .map_err(|_| "managed command broker contract rejected")?;
    require_grant_bound(&grants)?;
    runtime.recheck()?;
    Ok(ManagedProviderResources {
        provider,
        runtime,
        contract,
        grants,
        shell,
        handles,
    })
}

pub(crate) fn require_grant_bound(grants: &[SandboxResourceGrant]) -> Result<(), String> {
    let encoded = grants
        .iter()
        .map(|grant| {
            grant
                .root()
                .as_os_str()
                .to_string_lossy()
                .encode_utf16()
                .count()
                + 12
        })
        .sum::<usize>();
    if grants.len() > 1024 || encoded > 512 * 1024 {
        return Err("managed runtime closure exceeds the native launch bound".into());
    }
    Ok(())
}

pub(crate) fn verify_globalization_resource(
    helper: &PinnedSandboxHelper,
    path: &Path,
    digest: &str,
) -> Result<(), String> {
    globalization::verify(helper, path, digest)
}

pub(crate) fn pin(
    path: &Path,
    expected: &str,
    grants: &mut Vec<SandboxResourceGrant>,
    handles: &mut Vec<File>,
) -> Result<(), String> {
    if path.canonicalize().ok().as_deref() != Some(path) {
        return Err("runtime resource is missing or redirected".into());
    }
    let mut file = manifest::open_immutable(path)?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| "runtime resource is unreadable")?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    if hex::encode(hash.finalize()) != expected {
        return Err("runtime resource differs from its authenticated identity".into());
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|_| "runtime resource is unavailable")?;
    grants.push(SandboxResourceGrant::RuntimeReadOnly(path.to_path_buf()));
    handles.push(file);
    Ok(())
}

pub(crate) fn require_factory(provider: AgentChatProvider) -> Result<(), String> {
    match provider {
        AgentChatProvider::Claude | AgentChatProvider::Codex => {
            let helper = PinnedSandboxHelper::packaged()?;
            let (contract, _) = manifest::load(
                helper.runtime_root()?,
                provider.contained_contract().provider_id,
            )?;
            command_closure::require_command_closure(&contract)?;
            if option_env!("GENT_WINDOWS_GIT_BASH_SHA256").is_none() {
                return Err("reviewed managed credential/shell contract unavailable".into());
            }
            Ok(())
        }
        AgentChatProvider::Claurst => crate::windows_claurst_resources::require_factory(),
    }
}
