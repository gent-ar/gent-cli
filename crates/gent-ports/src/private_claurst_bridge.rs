use async_trait::async_trait;
use gent_types::{
    FrozenConversationContext, GoalContractError, GoalProjection, GoalRecord,
    NormalizedLifecycleSignal, NormalizedProviderEvent, PermissionCategory, PolicyRecord,
    SandboxEnforcement,
};

use crate::PortError;

pub const MAX_PRIVATE_CLAURST_DRAIN_FACTS: u16 = 64;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ClaurstSourceId(pub String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaurstSessionBinding {
    pub run_id: String,
    pub source_id: ClaurstSourceId,
    pub opaque_session_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaurstStartRequest {
    pub run_id: String,
    pub source_id: ClaurstSourceId,
    pub turn_id: String,
    pub prompt: String,
    pub context: FrozenConversationContext,
    pub attachments: Vec<ClaurstPromptAttachment>,
    pub goal: Option<ClaurstGoalProjection>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaurstPromptAttachment {
    pub display_name: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
}

impl ClaurstStartRequest {
    pub fn validate(&self) -> Result<(), GoalContractError> {
        if self.run_id.trim().is_empty()
            || self.source_id.0.trim().is_empty()
            || self.turn_id.trim().is_empty()
            || self.prompt.trim().is_empty()
            || self.context.conversation_id.0.trim().is_empty()
        {
            return Err(GoalContractError::InvalidMetadata);
        }
        if let Some(goal) = &self.goal
            && (goal.run_id != self.run_id
                || goal.source_id != self.source_id
                || *goal.goal.conversation_id() != self.context.conversation_id)
        {
            return Err(GoalContractError::InvalidMetadata);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaurstSubmitRequest {
    pub binding: ClaurstSessionBinding,
    pub turn_id: String,
    pub prompt: String,
    pub attachments: Vec<ClaurstPromptAttachment>,
    pub goal: Option<ClaurstGoalProjection>,
}

impl ClaurstSubmitRequest {
    pub fn validate(&self) -> Result<(), GoalContractError> {
        if self.binding.run_id.trim().is_empty()
            || self.binding.source_id.0.trim().is_empty()
            || self.binding.opaque_session_id.is_empty()
            || self.turn_id.trim().is_empty()
            || self.prompt.trim().is_empty()
        {
            return Err(GoalContractError::InvalidMetadata);
        }
        if let Some(goal) = &self.goal
            && (goal.run_id != self.binding.run_id || goal.source_id != self.binding.source_id)
        {
            return Err(GoalContractError::InvalidMetadata);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaurstGoalProjection {
    pub run_id: String,
    pub source_id: ClaurstSourceId,
    pub goal: GoalProjection,
}

impl ClaurstGoalProjection {
    pub fn from_active_goal(
        run_id: &str,
        source_id: ClaurstSourceId,
        goal: &GoalRecord,
    ) -> Result<Self, GoalContractError> {
        if source_id.0.is_empty() || run_id.trim().is_empty() {
            return Err(GoalContractError::InvalidMetadata);
        }
        let goal = GoalProjection::from_active(goal)?;
        Ok(Self {
            run_id: run_id.into(),
            source_id,
            goal,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaurstCheckpoint {
    pub run_id: String,
    pub source_id: ClaurstSourceId,
    pub cursor: u64,
    pub state_digest_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaurstNormalizedFact {
    pub source_id: ClaurstSourceId,
    pub cursor: u64,
    pub value: ClaurstFactValue,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClaurstFactValue {
    Event(NormalizedProviderEvent),
    Lifecycle(NormalizedLifecycleSignal),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaurstPermissionRequest {
    pub request_id: String,
    pub tool_use_id: String,
    pub tool_name: String,
    pub category: PermissionCategory,
    pub input: Option<serde_json::Value>,
    pub child_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaurstPermissionReply {
    AllowOnce,
    Deny,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaurstTerminal {
    Completed,
    Interrupted,
    Failed {
        classification: ClaurstFailureClassification,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaurstFailureClassification {
    Authentication,
    Authorization,
    Protocol,
    Unavailable,
    Internal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaurstDrainRequest {
    pub run_id: String,
    pub source_id: ClaurstSourceId,
    pub after_cursor: u64,
    pub limit: u16,
}

impl ClaurstDrainRequest {
    #[must_use]
    pub const fn is_bounded(&self) -> bool {
        self.limit > 0 && self.limit <= MAX_PRIVATE_CLAURST_DRAIN_FACTS
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaurstDrainBatch {
    pub facts: Vec<ClaurstNormalizedFact>,
    pub permissions: Vec<ClaurstPermissionRequest>,
    pub checkpoint: Option<ClaurstCheckpoint>,
    pub session_binding: Option<ClaurstSessionBinding>,
    pub terminal: Option<ClaurstTerminal>,
}

#[async_trait]
pub trait PrivateClaurstBridge: Send + Sync {
    async fn start(
        &self,
        request: ClaurstStartRequest,
    ) -> Result<ClaurstSessionBinding, PortError> {
        let _ = request;
        Err(PortError::Unavailable("private Claurst start".into()))
    }

    async fn bind_session(&self, binding: ClaurstSessionBinding) -> Result<(), PortError>;

    async fn submit(&self, request: ClaurstSubmitRequest) -> Result<(), PortError> {
        let _ = request;
        Err(PortError::Unavailable("private Claurst submit".into()))
    }

    async fn cancel(&self, _binding: ClaurstSessionBinding) -> Result<(), PortError> {
        Err(PortError::Unavailable("private Claurst cancel".into()))
    }

    async fn respond_permission(
        &self,
        _binding: ClaurstSessionBinding,
        _request_id: &str,
        _reply: ClaurstPermissionReply,
    ) -> Result<(), PortError> {
        Err(PortError::Unavailable(
            "private Claurst permission response".into(),
        ))
    }

    async fn permission_enforcement(
        &self,
        _binding: ClaurstSessionBinding,
        _policy: &PolicyRecord,
        _epoch: gent_types::HostEpoch,
    ) -> SandboxEnforcement {
        SandboxEnforcement::Unavailable
    }

    async fn owns_chat_tool(&self, _: &ClaurstSessionBinding, _: &str) -> bool {
        false
    }

    async fn runs_background_commands(&self, _: &ClaurstSessionBinding) -> bool {
        false
    }

    async fn drain(&self, request: ClaurstDrainRequest) -> Result<ClaurstDrainBatch, PortError>;
}

#[cfg(test)]
#[path = "private_claurst_bridge_tests.rs"]
mod tests;
