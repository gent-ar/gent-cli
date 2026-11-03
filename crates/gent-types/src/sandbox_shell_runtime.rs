use super::{
    SandboxLaunchContractError, SandboxLaunchProfile, SandboxNetworkPolicy, SandboxResourceGrant,
    valid_lock,
};
use crate::RunVersionLock;
use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxCommandEgress {
    BrokeredInternet,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxShellRuntime {
    lock: RunVersionLock,
    powershell: Option<RunVersionLock>,
    powershell_launcher: Option<RunVersionLock>,
    command_egress: Option<SandboxCommandEgress>,
}

impl SandboxShellRuntime {
    pub fn new(lock: RunVersionLock) -> Result<Self, SandboxLaunchContractError> {
        if lock.provider != "git-bash"
            || !valid_lock(&lock)
            || !Path::new(&lock.canonical_path).is_absolute()
            || !Path::new(&lock.canonical_path)
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("bash.exe"))
        {
            return Err(SandboxLaunchContractError::InvalidResourceGrant);
        }
        Ok(Self {
            lock,
            powershell: None,
            powershell_launcher: None,
            command_egress: None,
        })
    }

    pub fn lock(&self) -> &RunVersionLock {
        &self.lock
    }

    pub fn with_powershell(
        mut self,
        lock: RunVersionLock,
    ) -> Result<Self, SandboxLaunchContractError> {
        if lock.provider != "powershell7"
            || !valid_lock(&lock)
            || !Path::new(&lock.canonical_path).is_absolute()
            || !Path::new(&lock.canonical_path)
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("pwsh.exe"))
        {
            return Err(SandboxLaunchContractError::InvalidResourceGrant);
        }
        self.powershell = Some(lock);
        self.powershell_launcher = None;
        self.command_egress = None;
        Ok(self)
    }

    pub fn with_powershell_launcher(
        mut self,
        lock: RunVersionLock,
    ) -> Result<Self, SandboxLaunchContractError> {
        let engine = self
            .powershell
            .as_ref()
            .ok_or(SandboxLaunchContractError::InvalidResourceGrant)?;
        let engine_parent = Path::new(&engine.canonical_path)
            .parent()
            .ok_or(SandboxLaunchContractError::InvalidResourceGrant)?;
        let launcher = Path::new(&lock.canonical_path);
        let launcher_parent = launcher
            .parent()
            .ok_or(SandboxLaunchContractError::InvalidResourceGrant)?;
        let named = |path: &Path, name: &str| {
            path.file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case(name))
        };
        if lock.provider != "powershell7-launcher"
            || !valid_lock(&lock)
            || !launcher.is_absolute()
            || !named(launcher, "pwsh.exe")
            || !named(engine_parent, "powershell")
            || !named(launcher_parent, "powershell-launcher")
            || engine_parent.parent() != launcher_parent.parent()
        {
            return Err(SandboxLaunchContractError::InvalidResourceGrant);
        }
        self.powershell_launcher = Some(lock);
        self.command_egress = None;
        Ok(self)
    }

    pub fn with_brokered_command_egress(mut self) -> Result<Self, SandboxLaunchContractError> {
        if self.powershell_launcher.is_none() {
            return Err(SandboxLaunchContractError::MissingCommandEgress);
        }
        self.command_egress = Some(SandboxCommandEgress::BrokeredInternet);
        Ok(self)
    }

    pub const fn command_egress(&self) -> Option<SandboxCommandEgress> {
        self.command_egress
    }

    pub fn powershell_launcher_lock(&self) -> Option<&RunVersionLock> {
        self.powershell_launcher.as_ref()
    }

    pub fn powershell_lock(&self) -> Option<&RunVersionLock> {
        self.powershell.as_ref()
    }

    pub(super) fn covered_by(&self, resources: &[SandboxResourceGrant]) -> bool {
        if self.powershell.is_some() != self.powershell_launcher.is_some() {
            return false;
        }
        std::iter::once(&self.lock)
            .chain(self.powershell.iter())
            .chain(self.powershell_launcher.iter())
            .all(|lock| {
                resources.iter().any(|resource| {
                    !resource.writable()
                        && (Path::new(&lock.canonical_path) == resource.root()
                            || Path::new(&lock.canonical_path)
                                .parent()
                                .is_some_and(|parent| parent.starts_with(resource.root())))
                })
            })
    }
}

impl SandboxLaunchProfile {
    pub fn with_shell_runtime(
        mut self,
        shell: SandboxShellRuntime,
    ) -> Result<Self, SandboxLaunchContractError> {
        if !shell.covered_by(&self.resources) {
            return Err(SandboxLaunchContractError::InvalidResourceGrant);
        }
        self.shell_runtime = Some(shell);
        Ok(self)
    }

    pub fn shell_runtime(&self) -> Option<&SandboxShellRuntime> {
        self.shell_runtime.as_ref()
    }

    pub fn command_egress(&self) -> Option<SandboxCommandEgress> {
        self.gateway.as_ref()?;
        self.shell_runtime.as_ref()?.command_egress()
    }

    pub fn require_command_boundary(
        &self,
        registered_provider: bool,
    ) -> Result<(), SandboxLaunchContractError> {
        let broad = self
            .permission_binding
            .as_ref()
            .is_some_and(|binding| binding.policy().unattended_requires_sandbox());
        let isolated_worker = !registered_provider
            && self.network == SandboxNetworkPolicy::Disabled
            && self.gateway.is_none()
            && self.shell_runtime.is_none();
        if (broad || self.gateway.is_some()) && self.command_egress().is_none() && !isolated_worker
        {
            return Err(SandboxLaunchContractError::MissingCommandEgress);
        }
        if self.cpu_semantics == super::SandboxCpuSemantics::Uncapped && !isolated_worker {
            return Err(SandboxLaunchContractError::MissingResourceLimit);
        }
        Ok(())
    }
}
