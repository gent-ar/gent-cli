//! Durable, provider-neutral values for user-authored conversation goals.
//!
//! Goals are concise user intent records. They never contain provider output,
//! provider-native sessions, credentials, executable instructions, or plans.

use serde::{Deserialize, Serialize};

use crate::{AgentChatConversationId, DurableTurnPhase, ProviderFailureClassification};

const MAX_ID_BYTES: usize = 128;
pub const MAX_GOAL_OBJECTIVE_BYTES: usize = 4_096;
pub const MAX_GOAL_NOTE_BYTES: usize = 1_024;

/// Version of the durable goal value contract.
pub const GOAL_SCHEMA_VERSION: u16 = 2;

/// Immutable identity and ownership scope for one goal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoalBinding {
    pub goal_id: String,
    pub conversation_id: AgentChatConversationId,
}

/// Closed lifecycle state for a user-authored goal.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum GoalStatus {
    Active,
    Paused,
    Blocked,
    UsageLimited,
    BudgetLimited,
    Complete,
    Cleared,
}

impl GoalStatus {
    /// Returns whether pursuit of this goal has ended.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Complete | Self::Cleared)
    }

    #[must_use]
    pub const fn is_resumable(self) -> bool {
        matches!(
            self,
            Self::Paused | Self::Blocked | Self::UsageLimited | Self::BudgetLimited
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum GoalStatusReason {
    UserSet,
    UserPaused,
    UserStopped,
    UserResumed,
    UserCleared,
    UserReplaced,
    ModelCompleted,
    ModelBlocked,
    ProviderFailed,
    ProviderAuthentication,
    ProviderUsageLimit,
    ProviderContextLimit,
    AdmissionHeld,
    TokenBudgetExhausted,
    NoProgressLimit,
}

/// Immutable revisioned record representing one concise user goal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoalRecord {
    pub schema_version: u16,
    pub binding: GoalBinding,
    pub revision: u64,
    pub status: GoalStatus,
    pub reason: GoalStatusReason,
    pub objective: String,
    pub note: Option<String>,
    pub time_used_seconds: u64,
    pub active_since: Option<u64>,
    pub tokens_used: u64,
    pub token_budget: Option<u64>,
    pub turns_without_progress: u16,
    pub accounted_through_ordinal: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

impl GoalRecord {
    /// Validates bounded user-owned goal metadata before durable use.
    ///
    /// # Errors
    /// Returns an error for malformed identity, revision, or summary metadata.
    pub fn validate(&self) -> Result<(), GoalContractError> {
        if self.schema_version != GOAL_SCHEMA_VERSION
            || self.revision == 0
            || !valid_id(&self.binding.goal_id)
            || !valid_id(&self.binding.conversation_id.0)
            || !valid_text(&self.objective, MAX_GOAL_OBJECTIVE_BYTES)
            || self
                .note
                .as_deref()
                .is_some_and(|note| !valid_text(note, MAX_GOAL_NOTE_BYTES))
            || self.active_since.is_some() != (self.status == GoalStatus::Active)
            || self.token_budget == Some(0)
            || self.created_at > self.updated_at
        {
            return Err(GoalContractError::InvalidMetadata);
        }
        Ok(())
    }

    #[must_use]
    pub fn time_used_at(&self, now: u64) -> u64 {
        self.time_used_seconds.saturating_add(
            self.active_since
                .map_or(0, |since| now.saturating_sub(since)),
        )
    }

    #[must_use]
    pub fn budget_exhausted(&self) -> bool {
        self.token_budget
            .is_some_and(|budget| self.tokens_used >= budget)
    }
}

/// A validated active goal copied from the ledger into a provider adapter input.
///
/// This is deliberately not a client command or provider result. Only Gent may derive it from a
/// durable active [`GoalRecord`], and adapters may only use it as bounded context for a prompt.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalProjection {
    #[serde(skip)]
    conversation_id: AgentChatConversationId,
    goal_id: String,
    objective: String,
    tokens_used: u64,
    token_budget: Option<u64>,
}

impl GoalProjection {
    /// Copies one active, validated ledger goal into a provider-neutral adapter input.
    ///
    /// # Errors
    /// Returns an error if the record is invalid or no longer active.
    pub fn from_active(goal: &GoalRecord) -> Result<Self, GoalContractError> {
        goal.validate()?;
        if goal.status != GoalStatus::Active {
            return Err(GoalContractError::InactiveGoal);
        }
        Ok(Self {
            conversation_id: goal.binding.conversation_id.clone(),
            goal_id: goal.binding.goal_id.clone(),
            objective: goal.objective.clone(),
            tokens_used: goal.tokens_used,
            token_budget: goal.token_budget,
        })
    }

    #[must_use]
    pub const fn conversation_id(&self) -> &AgentChatConversationId {
        &self.conversation_id
    }

    #[must_use]
    pub fn goal_id(&self) -> &str {
        &self.goal_id
    }

    #[must_use]
    pub fn objective(&self) -> &str {
        &self.objective
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum GoalReportOutcome {
    Complete,
    Blocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GoalDispatchState {
    AwaitingReadiness,
    Pending,
    InFlight,
    Settled,
    Unprovable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoalTurnObservation {
    pub ordinal: u64,
    pub message_id: String,
    pub turn_id: String,
    pub run_id: String,
    pub receipt_id: String,
    pub phase: DurableTurnPhase,
    pub dispatch: GoalDispatchState,
    pub continuation_of: Option<String>,
    pub held: bool,
    pub tokens: u64,
    pub tool_calls: u32,
    pub failure: Option<ProviderFailureClassification>,
}

/// Contract validation failure that does not disclose provider data.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum GoalContractError {
    #[error("goal metadata is invalid or exceeds its bound")]
    InvalidMetadata,
    #[error("only an active goal may be projected to a provider adapter")]
    InactiveGoal,
}

pub fn valid_goal_id(value: &str) -> bool {
    valid_id(value)
}

pub fn valid_goal_text(value: &str, limit: usize) -> bool {
    valid_text(value, limit)
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_ID_BYTES && !value.chars().any(char::is_control)
}

fn valid_text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.len() <= limit && !value.contains('\0')
}

#[cfg(test)]
#[path = "goal_tests.rs"]
mod tests;
