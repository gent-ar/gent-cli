use crate::{HostEpoch, PolicyRecord, SandboxLaunchContractError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxPermissionBinding {
    conversation_id: String,
    run_id: String,
    policy: PolicyRecord,
    host_epoch: HostEpoch,
}

impl SandboxPermissionBinding {
    pub fn new(
        conversation_id: String,
        run_id: String,
        policy: PolicyRecord,
        host_epoch: HostEpoch,
    ) -> Result<Self, SandboxLaunchContractError> {
        if [
            &conversation_id,
            &run_id,
            &policy.workspace_id,
            &policy.policy_id,
        ]
        .iter()
        .any(|value| value.trim().is_empty())
            || policy.revision == 0
        {
            return Err(SandboxLaunchContractError::InvalidResourceGrant);
        }
        Ok(Self {
            conversation_id,
            run_id,
            policy,
            host_epoch,
        })
    }

    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }
    pub fn run_id(&self) -> &str {
        &self.run_id
    }
    pub fn policy(&self) -> &PolicyRecord {
        &self.policy
    }
    pub const fn host_epoch(&self) -> HostEpoch {
        self.host_epoch
    }

    pub(super) fn digest_material(&self) -> String {
        serde_json::to_string(&(
            &self.conversation_id,
            &self.run_id,
            &self.policy,
            self.host_epoch,
        ))
        .expect("serializable permission binding")
    }
}
