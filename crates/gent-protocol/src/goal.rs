//! Capability-gated IPC for durable, provider-neutral user goals.
//!
//! This finite contract only carries user-owned goal records and revision-fenced
//! transitions. It neither invokes a provider nor grants goal authority; an
//! observer daemon must not advertise this capability.

use gent_types::{
    AgentChatConversationId, GoalRecord, GoalReportOutcome, MAX_GOAL_NOTE_BYTES,
    MAX_GOAL_OBJECTIVE_BYTES, valid_goal_id, valid_goal_text,
};
use serde::{Deserialize, Serialize};

/// Required before a client may create, settle, or read durable `/goal` records.
pub const GOAL_CAPABILITY: &str = "goal-v1";
/// Maximum encoded size of one goal endpoint frame.
pub const MAX_GOAL_FRAME_BYTES: usize = 64 * 1024;

/// One finite, correlated goal exchange on its dedicated local endpoint.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum GoalFrame {
    Set {
        request_id: String,
        conversation_id: AgentChatConversationId,
        objective: String,
        token_budget: Option<u64>,
    },
    Pause {
        request_id: String,
        conversation_id: AgentChatConversationId,
        goal_id: String,
        expected_revision: u64,
    },
    Resume {
        request_id: String,
        conversation_id: AgentChatConversationId,
        goal_id: String,
        expected_revision: u64,
    },
    Clear {
        request_id: String,
        conversation_id: AgentChatConversationId,
        goal_id: String,
        expected_revision: u64,
    },
    Read {
        request_id: String,
        conversation_id: AgentChatConversationId,
    },
    Report {
        request_id: String,
        goal_id: String,
        outcome: GoalReportOutcome,
        note: Option<String>,
    },
    Goal {
        request_id: String,
        conversation_id: AgentChatConversationId,
        goal: Option<GoalRecord>,
    },
    Rejected {
        request_id: String,
        code: GoalRejectionCode,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum GoalRejectionCode {
    GoalInvalid,
    GoalMissing,
    GoalRevisionMismatch,
    GoalNotActive,
    GoalNotResumable,
    GoalBudgetExhausted,
    GoalNoActiveTurn,
}

impl GoalFrame {
    #[must_use]
    pub fn request_id(&self) -> &str {
        match self {
            Self::Set { request_id, .. }
            | Self::Pause { request_id, .. }
            | Self::Resume { request_id, .. }
            | Self::Clear { request_id, .. }
            | Self::Read { request_id, .. }
            | Self::Report { request_id, .. }
            | Self::Goal { request_id, .. }
            | Self::Rejected { request_id, .. } => request_id,
        }
    }

    /// Validates bounded correlation and goal values before dedicated transport.
    ///
    /// # Errors
    /// Returns an error for malformed, mismatched, or oversized public frames.
    pub fn validate(&self) -> Result<(), GoalFrameError> {
        valid_id(self.request_id())?;
        match self {
            Self::Set {
                conversation_id,
                objective,
                token_budget,
                ..
            } => {
                valid_id(&conversation_id.0)?;
                if !valid_goal_text(objective, MAX_GOAL_OBJECTIVE_BYTES) || *token_budget == Some(0)
                {
                    return Err(GoalFrameError::InvalidValue);
                }
            }
            Self::Pause {
                conversation_id,
                goal_id,
                expected_revision,
                ..
            }
            | Self::Resume {
                conversation_id,
                goal_id,
                expected_revision,
                ..
            }
            | Self::Clear {
                conversation_id,
                goal_id,
                expected_revision,
                ..
            } => {
                valid_id(&conversation_id.0)?;
                valid_id(goal_id)?;
                if *expected_revision == 0 {
                    return Err(GoalFrameError::InvalidValue);
                }
            }
            Self::Read {
                conversation_id, ..
            } => valid_id(&conversation_id.0)?,
            Self::Report { goal_id, note, .. } => {
                valid_id(goal_id)?;
                if note
                    .as_deref()
                    .is_some_and(|note| !valid_goal_text(note, MAX_GOAL_NOTE_BYTES))
                {
                    return Err(GoalFrameError::InvalidValue);
                }
            }
            Self::Goal {
                conversation_id,
                goal,
                ..
            } => {
                valid_id(&conversation_id.0)?;
                if let Some(goal) = goal {
                    goal.validate()?;
                    if goal.binding.conversation_id != *conversation_id {
                        return Err(GoalFrameError::BindingMismatch);
                    }
                }
            }
            Self::Rejected { .. } => {}
        }
        if self.encoded_len()? > MAX_GOAL_FRAME_BYTES {
            return Err(GoalFrameError::TooLarge);
        }
        Ok(())
    }

    #[must_use]
    pub const fn is_client_request(&self) -> bool {
        !matches!(self, Self::Goal { .. } | Self::Rejected { .. })
    }

    fn encoded_len(&self) -> Result<usize, GoalFrameError> {
        serde_json::to_vec(self)
            .map(|encoded| encoded.len())
            .map_err(|_| GoalFrameError::InvalidEncoding)
    }
}

fn valid_id(value: &str) -> Result<(), GoalFrameError> {
    valid_goal_id(value)
        .then_some(())
        .ok_or(GoalFrameError::InvalidIdentifier)
}

/// Value-free validation errors for the goal endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum GoalFrameError {
    #[error("goal correlation identifier is invalid")]
    InvalidIdentifier,
    #[error("goal value is invalid")]
    InvalidValue,
    #[error("goal response binding does not match its request scope")]
    BindingMismatch,
    #[error("goal frame exceeds byte budget")]
    TooLarge,
    #[error("goal frame could not be encoded")]
    InvalidEncoding,
}

impl From<gent_types::GoalContractError> for GoalFrameError {
    fn from(_: gent_types::GoalContractError) -> Self {
        Self::InvalidValue
    }
}

#[cfg(test)]
#[path = "goal_tests.rs"]
mod tests;
