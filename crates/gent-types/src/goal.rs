//! Durable, provider-neutral values for user-authored conversation goals.
//!
//! Goals are concise user intent records. They never contain provider output,
//! provider-native sessions, credentials, executable instructions, or plans.

use serde::{Deserialize, Serialize};

use crate::{AgentChatConversationId, AgentChatRunId, HostEpoch};

const MAX_ID_BYTES: usize = 128;
const MAX_SUMMARY_BYTES: usize = 1_024;

/// Version of the durable goal value contract.
pub const GOAL_SCHEMA_VERSION: u16 = 1;

/// Immutable identity and ownership scope for one goal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoalBinding {
    pub goal_id: String,
    pub conversation_id: AgentChatConversationId,
    pub run_id: AgentChatRunId,
}

/// Closed lifecycle state for a user-authored goal.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum GoalStatus {
    Active,
    Completed,
    Abandoned,
    Failed,
}

impl GoalStatus {
    /// Returns whether this state can no longer transition.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Active)
    }
}

/// Immutable revisioned record representing one concise user goal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoalRecord {
    pub schema_version: u16,
    pub binding: GoalBinding,
    pub revision: u64,
    pub status: GoalStatus,
    pub summary: String,
}

/// A validated active goal copied from the ledger into a provider adapter input.
///
/// This is deliberately not a client command or provider result. Only Gent may derive it from a
/// durable active [`GoalRecord`], and adapters may only use it as bounded context for a prompt.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalProjection {
    binding: GoalBinding,
    revision: u64,
    summary: String,
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
            binding: goal.binding.clone(),
            revision: goal.revision,
            summary: goal.summary.clone(),
        })
    }

    /// Returns the immutable goal ownership binding.
    #[must_use]
    pub const fn binding(&self) -> &GoalBinding {
        &self.binding
    }

    /// Returns the exact durable revision selected by Gent.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns the bounded user-authored goal summary.
    #[must_use]
    pub fn summary(&self) -> &str {
        &self.summary
    }
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
            || !valid_id(&self.binding.run_id.0)
            || !valid_summary(&self.summary)
        {
            return Err(GoalContractError::InvalidMetadata);
        }
        Ok(())
    }
}

/// A revision-fenced request to settle an active goal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoalTransition {
    pub binding: GoalBinding,
    pub expected_revision: u64,
    /// Writer fence rechecked by the daemon; it is not durable goal identity.
    pub host_epoch: HostEpoch,
    pub next_status: GoalStatus,
}

impl GoalTransition {
    /// Validates the transition's correlation metadata.
    ///
    /// # Errors
    /// Returns an error when its identity or expected revision is invalid.
    pub fn validate(&self) -> Result<(), GoalContractError> {
        if self.expected_revision == 0
            || !valid_id(&self.binding.goal_id)
            || !valid_id(&self.binding.conversation_id.0)
            || !valid_id(&self.binding.run_id.0)
        {
            return Err(GoalContractError::InvalidMetadata);
        }
        Ok(())
    }
}

/// Contract validation failure that does not disclose provider data.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum GoalContractError {
    #[error("goal metadata is invalid or exceeds its bound")]
    InvalidMetadata,
    #[error("only an active goal may be projected to a provider adapter")]
    InactiveGoal,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_ID_BYTES && !value.chars().any(char::is_control)
}

fn valid_summary(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_SUMMARY_BYTES && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        GOAL_SCHEMA_VERSION, GoalBinding, GoalContractError, GoalProjection, GoalRecord,
        GoalStatus, GoalTransition,
    };
    use crate::{AgentChatConversationId, AgentChatRunId};

    fn binding() -> GoalBinding {
        GoalBinding {
            goal_id: "goal-1".into(),
            conversation_id: AgentChatConversationId("conversation-1".into()),
            run_id: AgentChatRunId("run-1".into()),
        }
    }

    #[test]
    fn goal_record_has_a_closed_public_shape() {
        let record = GoalRecord {
            schema_version: GOAL_SCHEMA_VERSION,
            binding: binding(),
            revision: 1,
            status: GoalStatus::Active,
            summary: "Ship the terminal workflow".into(),
        };
        assert_eq!(
            serde_json::to_value(record).unwrap(),
            json!({
                "schemaVersion": 1,
                "binding": { "goalId": "goal-1", "conversationId": "conversation-1", "runId": "run-1" },
                "revision": 1,
                "status": "active",
                "summary": "Ship the terminal workflow"
            })
        );
    }

    #[test]
    fn goal_values_reject_provider_fields_and_invalid_terminal_transition() {
        let value = json!({
            "binding": { "goalId": "goal-1", "conversationId": "conversation-1", "runId": "run-1" },
            "expectedRevision": 1,
            "hostEpoch": 1,
            "nextStatus": "completed",
            "providerSessionId": "must-not-cross-the-contract"
        });
        assert!(serde_json::from_value::<GoalTransition>(value).is_err());
        assert!(GoalStatus::Completed.is_terminal());
    }

    #[test]
    fn only_a_valid_active_ledger_goal_can_be_projected_to_an_adapter() {
        let active = GoalRecord {
            schema_version: GOAL_SCHEMA_VERSION,
            binding: binding(),
            revision: 4,
            status: GoalStatus::Active,
            summary: "Complete the safe task".into(),
        };
        let projection = GoalProjection::from_active(&active).unwrap();
        assert_eq!(projection.binding(), &active.binding);
        assert_eq!(projection.revision(), 4);
        assert_eq!(projection.summary(), "Complete the safe task");

        let terminal = GoalRecord {
            status: GoalStatus::Completed,
            ..active
        };
        assert_eq!(
            GoalProjection::from_active(&terminal),
            Err(GoalContractError::InactiveGoal)
        );
    }
}
