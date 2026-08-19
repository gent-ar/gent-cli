//! Path-free containment policy resolved into one run-specific sandbox profile.

use std::path::Path;

use crate::{
    AgentChatMode, SandboxLaunchContractError, SandboxLaunchProfile, SandboxNetworkPolicy,
    SandboxResourceLimits,
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

/// Reusable, path-free containment policy retained by a private daemon composition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxLaunchPolicy {
    inherited_environment: Vec<String>,
    network: SandboxNetworkPolicy,
    limits: SandboxResourceLimits,
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
        })
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
        SandboxLaunchProfile::new(
            workspace_root,
            &[root],
            &writable,
            self.inherited_environment.clone(),
            self.network.clone(),
            self.limits,
        )
    }
}
