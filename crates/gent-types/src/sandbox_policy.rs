use std::path::Path;

use crate::{
    AgentChatMode, SandboxLaunchContractError, SandboxLaunchProfile, SandboxNetworkPolicy,
    SandboxPrivateState, SandboxResourceGrant, SandboxResourceLimits,
};

/// The filesystem authority granted to one provider process by a durable Gent mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxWorkspaceAccess {
    ReadOnly,
    ReadWrite,
}

impl SandboxWorkspaceAccess {
    /// Converts the durable user-visible mode into its containment authority.
    #[must_use]
    pub const fn from_mode(mode: AgentChatMode) -> Self {
        match mode {
            AgentChatMode::Ask | AgentChatMode::Plan => Self::ReadOnly,
            AgentChatMode::Agent => Self::ReadWrite,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxLaunchPolicy {
    inherited_environment: Vec<String>,
    network: SandboxNetworkPolicy,
    limits: SandboxResourceLimits,
    cpu_semantics: crate::SandboxCpuSemantics,
    resources: Vec<SandboxResourceGrant>,
    private_state: Option<SandboxPrivateState>,
    permission_binding: Option<crate::SandboxPermissionBinding>,
    shell_runtime: Option<crate::SandboxShellRuntime>,
}

impl SandboxLaunchPolicy {
    /// Validates immutable containment settings without binding them to one workspace.
    ///
    /// # Errors
    /// Returns an error when environment, network, or resource fields violate the sandbox
    /// contract.
    pub fn new(
        inherited_environment: Vec<String>,
        network: SandboxNetworkPolicy,
        limits: SandboxResourceLimits,
    ) -> Result<Self, SandboxLaunchContractError> {
        SandboxLaunchProfile::validate_policy(&inherited_environment, &network, limits)?;
        Ok(Self {
            inherited_environment,
            network,
            limits,
            cpu_semantics: crate::SandboxCpuSemantics::StrictTotal,
            resources: Vec::new(),
            private_state: None,
            permission_binding: None,
            shell_runtime: None,
        })
    }

    pub fn with_cpu_semantics(
        mut self,
        value: crate::SandboxCpuSemantics,
    ) -> Result<Self, SandboxLaunchContractError> {
        self.cpu_semantics = value.validate()?;
        Ok(self)
    }

    #[must_use]
    pub fn with_resources(mut self, resources: Vec<SandboxResourceGrant>) -> Self {
        self.resources = resources;
        self
    }

    #[must_use]
    pub fn with_private_state(mut self, state: SandboxPrivateState) -> Self {
        self.private_state = Some(state);
        self
    }

    #[must_use]
    pub fn with_permission_binding(mut self, binding: crate::SandboxPermissionBinding) -> Self {
        self.permission_binding = Some(binding);
        self
    }

    #[must_use]
    pub fn with_shell_runtime(mut self, shell: crate::SandboxShellRuntime) -> Self {
        self.shell_runtime = Some(shell);
        self
    }

    /// Derives a launch profile from the durable run workspace and mode authority.
    ///
    /// # Errors
    /// Returns an error when the workspace is not a canonical safe root.
    pub fn profile_for_workspace(
        &self,
        workspace_root: &Path,
        access: SandboxWorkspaceAccess,
    ) -> Result<SandboxLaunchProfile, SandboxLaunchContractError> {
        let root = workspace_root.to_path_buf();
        let writable = matches!(access, SandboxWorkspaceAccess::ReadWrite)
            .then_some(root.clone())
            .into_iter()
            .collect::<Vec<_>>();
        let profile = SandboxLaunchProfile::new(
            workspace_root,
            &[root],
            &writable,
            self.inherited_environment.clone(),
            self.network.clone(),
            self.limits,
        )?
        .with_resources(self.resources.clone())?
        .with_cpu_semantics(self.cpu_semantics)?;
        let profile = match &self.permission_binding {
            Some(binding) => profile.with_permission_binding(binding.clone()),
            None => profile,
        };
        let profile = match &self.private_state {
            Some(state) => profile.with_private_state(state.clone()),
            None => Ok(profile),
        }?;
        match &self.shell_runtime {
            Some(shell) => profile.with_shell_runtime(shell.clone()),
            None => Ok(profile),
        }
    }
}
