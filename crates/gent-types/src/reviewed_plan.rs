use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    AgentChatConversationId, AgentChatRequestId, AgentChatRunId, AgentChatSelection, HostEpoch,
    Receipt, ReceiptId,
};

const MAX_ID_BYTES: usize = 128;
pub const MAX_PLAN_CONTENT_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ReviewedPlanId(pub String);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct PlanRevision(pub u64);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum PlanStatus {
    Draft,
    ReadyForReview,
    Approved,
    Rejected,
    Superseded,
    TerminallyFailed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanArtifact {
    pub plan_id: ReviewedPlanId,
    pub conversation_id: AgentChatConversationId,
    pub source_run_id: AgentChatRunId,
    pub source_turn_id: String,
    pub revision: PlanRevision,
    pub content_digest_sha256: String,
    pub status: PlanStatus,
    pub content: String,
}

impl PlanArtifact {
    #[must_use]
    pub fn content_digest(content: &str) -> String {
        format!("{:x}", Sha256::digest(content.as_bytes()))
    }

    /// # Errors
    /// Returns an error for malformed IDs, an unbounded plan, or a digest that is not the plan's.
    pub fn validate(&self) -> Result<(), ReviewedPlanContractError> {
        valid_id(&self.plan_id.0)?;
        valid_id(&self.conversation_id.0)?;
        valid_id(&self.source_run_id.0)?;
        valid_id(&self.source_turn_id)?;
        if self.content.trim().is_empty() || self.content.len() > MAX_PLAN_CONTENT_BYTES {
            return Err(ReviewedPlanContractError::InvalidMetadata);
        }
        if self.revision.0 == 0
            || !valid_digest(&self.content_digest_sha256)
            || self.content_digest_sha256 != Self::content_digest(&self.content)
        {
            return Err(ReviewedPlanContractError::InvalidRevisionOrDigest);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanTurn {
    pub conversation_id: AgentChatConversationId,
    pub run_id: AgentChatRunId,
    pub turn_id: String,
    pub content: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanImplementation {
    pub idempotency_key: String,
    pub implementation_run_id: AgentChatRunId,
    pub plan: PlanArtifact,
}

/// Whether a child run receives prior provider-neutral history.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum ContextPolicy {
    Preserve,
    Clear,
}

/// Idempotent approval of one exact reviewed-plan revision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartImplementationRequest {
    pub request_id: AgentChatRequestId,
    pub receipt_id: ReceiptId,
    pub idempotency_key: String,
    pub host_epoch: HostEpoch,
    pub policy_workspace_id: String,
    pub policy_revision: u64,
    pub conversation_id: AgentChatConversationId,
    pub plan_id: ReviewedPlanId,
    pub plan_revision: PlanRevision,
    pub plan_content_digest_sha256: String,
    pub parent_run_id: AgentChatRunId,
    pub selection: AgentChatSelection,
    pub context_policy: ContextPolicy,
}

impl StartImplementationRequest {
    /// Validates the exact approval boundary without starting any provider work.
    ///
    /// # Errors
    /// Returns an error for malformed correlation, plan identity, digest, or selection model.
    pub fn validate(&self) -> Result<(), ReviewedPlanContractError> {
        valid_id(&self.request_id.0)?;
        valid_id(&self.receipt_id.0)?;
        valid_id(&self.idempotency_key)?;
        valid_id(&self.policy_workspace_id)?;
        valid_id(&self.conversation_id.0)?;
        valid_id(&self.plan_id.0)?;
        valid_id(&self.parent_run_id.0)?;
        if self.policy_revision == 0
            || self.plan_revision.0 == 0
            || !valid_digest(&self.plan_content_digest_sha256)
        {
            return Err(ReviewedPlanContractError::InvalidRevisionOrDigest);
        }
        valid_id(&self.selection.model)
    }
}

/// Durable result of an accepted approval and immutable child-run reservation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartImplementationResult {
    pub receipt: Receipt,
    pub conversation_id: AgentChatConversationId,
    pub plan_id: ReviewedPlanId,
    pub plan_revision: PlanRevision,
    pub parent_run_id: AgentChatRunId,
    pub implementation_run_id: AgentChatRunId,
    pub selection: AgentChatSelection,
    pub context_policy: ContextPolicy,
    pub context_through_ordinal: u64,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ReviewedPlanContractError {
    #[error("reviewed-plan metadata is invalid or exceeds its bound")]
    InvalidMetadata,
    #[error("reviewed-plan revision or digest is invalid")]
    InvalidRevisionOrDigest,
}

fn valid_id(value: &str) -> Result<(), ReviewedPlanContractError> {
    valid_bounded(value, MAX_ID_BYTES)
}

fn valid_bounded(value: &str, max: usize) -> Result<(), ReviewedPlanContractError> {
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(ReviewedPlanContractError::InvalidMetadata);
    }
    Ok(())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
#[path = "reviewed_plan_tests.rs"]
mod tests;
