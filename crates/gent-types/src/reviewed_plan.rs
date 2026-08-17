//! Secret-free, provider-neutral values for reviewed-plan implementation.
//!
//! These values describe a future authority flow. They never contain raw
//! provider events, hidden reasoning, provider session identifiers, patches,
//! commands, credentials, or endpoint configuration.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    AgentChatConversationId, AgentChatRequestId, AgentChatRunId, AgentChatSelection, HostEpoch,
    PermissionCategory, Receipt, ReceiptId,
};

const MAX_ID_BYTES: usize = 128;
const MAX_SUMMARY_BYTES: usize = 1_024;
const MAX_PATH_BYTES: usize = 4_096;
const MAX_ITEMS: usize = 100;

/// An immutable reviewed-plan identifier.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ReviewedPlanId(pub String);

/// A monotonically increasing immutable revision of one reviewed plan.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct PlanRevision(pub u64);

/// Closed lifecycle states for a provider-normalized plan.
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

/// A normalized class of planned work, never a provider command or shell text.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum PlanActionKind {
    Read,
    Edit,
    Command,
    Git,
    Mcp,
}

/// One concise, provider-neutral action proposed by a reviewed plan.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanAction {
    pub action_id: String,
    pub kind: PlanActionKind,
    pub summary: String,
}

/// A normalized class of potential implementation risk.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum PlanRiskKind {
    DataLoss,
    Security,
    Dependency,
    ExternalSideEffect,
    Unknown,
}

/// Relative severity of a concise, user-visible plan risk.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum PlanRiskSeverity {
    Low,
    Medium,
    High,
}

/// One concise normalized risk, without hidden provider reasoning.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanRisk {
    pub kind: PlanRiskKind,
    pub severity: PlanRiskSeverity,
    pub summary: String,
}

/// The file-level effect proposed by a plan, without a raw diff or patch.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum PlanDiffKind {
    Add,
    Modify,
    Delete,
}

/// A normalized, path-level change preview without source content.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanDiff {
    pub path: String,
    pub kind: PlanDiffKind,
    pub summary: String,
}

/// A concise preview of a provider-neutral permission a plan may request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanPermissionPreview {
    pub category: PermissionCategory,
    pub summary: String,
}

/// Immutable, normalized content presented to a user for plan review.
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
    pub actions: Vec<PlanAction>,
    pub risks: Vec<PlanRisk>,
    pub diffs: Vec<PlanDiff>,
    pub permission_preview: Vec<PlanPermissionPreview>,
}

impl PlanArtifact {
    /// Validates bounded public metadata before durable or protocol use.
    ///
    /// # Errors
    /// Returns an error for malformed IDs, digests, paths, or bounded review fields.
    pub fn validate(&self) -> Result<(), ReviewedPlanContractError> {
        valid_id(&self.plan_id.0)?;
        valid_id(&self.conversation_id.0)?;
        valid_id(&self.source_run_id.0)?;
        valid_id(&self.source_turn_id)?;
        if self.revision.0 == 0 || !valid_digest(&self.content_digest_sha256) {
            return Err(ReviewedPlanContractError::InvalidRevisionOrDigest);
        }
        valid_items(&self.actions, |item| {
            valid_id(&item.action_id).and_then(|()| valid_summary(&item.summary))
        })?;
        valid_items(&self.risks, |item| valid_summary(&item.summary))?;
        valid_items(&self.diffs, |item| {
            valid_path(&item.path).and_then(|()| valid_summary(&item.summary))
        })?;
        valid_items(&self.permission_preview, |item| {
            valid_summary(&item.summary)
        })
    }
}

/// Whether a child implementation run receives prior normalized history.
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

fn valid_items<T>(
    items: &[T],
    validator: impl Fn(&T) -> Result<(), ReviewedPlanContractError>,
) -> Result<(), ReviewedPlanContractError> {
    if items.len() > MAX_ITEMS {
        return Err(ReviewedPlanContractError::InvalidMetadata);
    }
    items.iter().try_for_each(validator)
}

fn valid_id(value: &str) -> Result<(), ReviewedPlanContractError> {
    valid_bounded(value, MAX_ID_BYTES)
}

fn valid_summary(value: &str) -> Result<(), ReviewedPlanContractError> {
    valid_bounded(value, MAX_SUMMARY_BYTES)
}

fn valid_path(value: &str) -> Result<(), ReviewedPlanContractError> {
    valid_bounded(value, MAX_PATH_BYTES)
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
