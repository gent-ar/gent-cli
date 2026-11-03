use crate::{
    windows_provider_resources::{
        manifest::{self, ResourceRoot},
        pin,
    },
    windows_sandbox_runtime::PinnedSandboxHelper,
};
use gent_types::{AgentChatProvider, RunVersionLock, SandboxResourceGrant, SandboxShellRuntime};
use std::{fs::File, path::Path};
#[path = "windows_claurst_powershell.rs"]
mod powershell;

pub(crate) struct ClaurstResources {
    lock: RunVersionLock,
    grants: Vec<SandboxResourceGrant>,
    shell: SandboxShellRuntime,
    handles: Vec<File>,
}
impl ClaurstResources {
    pub(crate) fn grants(&self) -> Vec<SandboxResourceGrant> {
        self.grants.clone()
    }
    pub(crate) fn shell(&self) -> SandboxShellRuntime {
        self.shell.clone()
    }
    pub(crate) fn into_handles(self) -> Vec<File> {
        self.handles
    }
}
impl crate::permission_backend_registry::VerifiedProviderFactoryResources for ClaurstResources {
    fn provider(&self) -> AgentChatProvider {
        AgentChatProvider::Claurst
    }
    fn executable_lock(&self) -> &RunVersionLock {
        &self.lock
    }
    fn command_egress(&self) -> Option<gent_types::SandboxCommandEgress> {
        self.shell.command_egress()
    }
    fn recheck(&self) -> Result<(), String> {
        if gent_drivers::lock::rechecked_identity(&self.lock)
            .map_err(|_| "Claurst image unavailable")?
            != self.lock
        {
            return Err("Claurst image changed".into());
        }
        Ok(())
    }
}

pub(crate) fn require_factory() -> Result<(), String> {
    let helper = PinnedSandboxHelper::packaged()?;
    let (contract, _) = manifest::load(helper.runtime_root()?, "claurst")?;
    if !AgentChatProvider::Claurst
        .contained_contract()
        .credential_contracts
        .contains(&contract.credential_contract.as_str())
        || option_env!("GENT_WINDOWS_GIT_BASH_SHA256").is_none()
    {
        return Err("reviewed Claurst model/shell contract unavailable".into());
    }
    powershell::require_contract(&contract)
}

pub(crate) fn resources(
    helper: &PinnedSandboxHelper,
    lock: &RunVersionLock,
) -> Result<ClaurstResources, String> {
    let root = helper.runtime_root()?;
    let (contract, manifest_handle) = manifest::load(root, "claurst")?;
    if lock.provider != "claurst"
        || contract.executable_sha256.as_deref() != Some(lock.digest_sha256.as_str())
        || gent_adapters::provider_platform::host_platform()
            .is_none_or(|platform| platform.npm != contract.platform)
    {
        return Err("Claurst executable does not match its reviewed runtime contract".into());
    }
    let mut grants = Vec::new();
    let mut handles = vec![manifest_handle];
    let executable = Path::new(&lock.canonical_path);
    pin(executable, &lock.digest_sha256, &mut grants, &mut handles)?;
    let helper_lock = helper.image_lock();
    pin(
        Path::new(&helper_lock.canonical_path),
        &helper_lock.digest_sha256,
        &mut grants,
        &mut handles,
    )?;
    let package_root = executable
        .parent()
        .ok_or("Claurst image directory unavailable")?;
    let git_root = root.join("git");
    manifest::require_git_mounts(&contract)?;
    let shell_path = manifest::resource_path(&git_root, "usr/bin/bash.exe")?;
    let mut shell_present = false;
    powershell::require_contract(&contract)?;
    let os_names: Vec<String> = contract
        .os_libraries
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect();
    let os = crate::windows_os_resources::verify_os_libraries(helper, &os_names)?;
    let windows_root = os
        .system_directory()
        .parent()
        .ok_or("verified Windows directory unavailable")?;
    let powershell_root = root.join("powershell");
    let powershell_launcher_root = root.join("powershell-launcher");
    for resource in &contract.files {
        let directory = match resource.root {
            ResourceRoot::Package => package_root,
            ResourceRoot::GitRuntime => &git_root,
            ResourceRoot::PowerShellRuntime => &powershell_root,
            ResourceRoot::PowerShellLauncher => &powershell_launcher_root,
            ResourceRoot::OsGlobalizationData => windows_root,
            ResourceRoot::NodeRuntime => {
                return Err("Claurst runtime cannot grant a Node dependency".into());
            }
        };
        let path = manifest::resource_path(directory, &resource.path)?;
        shell_present |= path == shell_path;
        pin(&path, &resource.sha256, &mut grants, &mut handles)?;
        if resource.root == ResourceRoot::OsGlobalizationData {
            crate::windows_provider_resources::verify_globalization_resource(
                helper,
                &path,
                &resource.sha256,
            )?;
        }
    }
    if !shell_present {
        return Err("Claurst Git shell dependency closure is incomplete".into());
    }
    let shell_lock =
        gent_drivers::lock::capture("git-bash", &shell_path, "5.2.15", "managed-git-bash-v1")
            .map_err(|_| "reviewed Git shell unavailable")?;
    if Some(shell_lock.digest_sha256.as_str()) != option_env!("GENT_WINDOWS_GIT_BASH_SHA256") {
        return Err("reviewed Git shell changed".into());
    }
    grants.extend(
        os.paths()
            .into_iter()
            .map(SandboxResourceGrant::RuntimeReadOnly),
    );
    handles.extend(os.into_handles());
    crate::windows_provider_resources::require_grant_bound(&grants)?;
    let shell = SandboxShellRuntime::new(shell_lock).map_err(|_| "reviewed Git shell rejected")?;
    let shell = powershell::attach(root, &contract, shell)?;
    Ok(ClaurstResources {
        lock: lock.clone(),
        grants,
        shell,
        handles,
    })
}
