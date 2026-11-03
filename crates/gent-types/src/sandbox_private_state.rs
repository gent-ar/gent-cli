use std::path::{Component, Path, PathBuf};

use super::{SandboxLaunchContractError, SandboxLaunchProfile, SandboxResourceGrant};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxPrivateState {
    run_id: String,
    root: PathBuf,
}

impl SandboxPrivateState {
    pub fn new(run_id: String, root: PathBuf) -> Result<Self, SandboxLaunchContractError> {
        if run_id.is_empty()
            || run_id.len() > 160
            || !run_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
            || !root.is_absolute()
            || root.parent().is_none()
            || root
                .components()
                .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
        {
            return Err(SandboxLaunchContractError::InvalidResourceGrant);
        }
        Ok(Self { run_id, root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn run_id(&self) -> &str {
        &self.run_id
    }
    pub fn directories(&self) -> Vec<PathBuf> {
        ["home", "appdata", "localappdata", "tmp", "claude", "codex"]
            .iter()
            .map(|child| self.root.join(child))
            .collect()
    }
}

impl SandboxLaunchProfile {
    pub fn with_private_state(
        mut self,
        state: SandboxPrivateState,
    ) -> Result<Self, SandboxLaunchContractError> {
        if !self.resources.iter().any(|grant| {
            matches!(grant,
            SandboxResourceGrant::PrivateStateReadWrite(root) if root == state.root())
        }) {
            return Err(SandboxLaunchContractError::InvalidResourceGrant);
        }
        self.private_state = Some(state);
        Ok(self)
    }
    pub fn private_state(&self) -> Option<&SandboxPrivateState> {
        self.private_state.as_ref()
    }
}
