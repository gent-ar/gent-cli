//! Capability-gated IPC for durable, provider-neutral user goals.
//!
//! This finite contract only carries user-owned goal records and revision-fenced
//! transitions. It neither invokes a provider nor grants goal authority; an
//! observer daemon must not advertise this capability.

use gent_types::{AgentChatConversationId, GoalBinding, GoalRecord, GoalTransition};
use serde::{Deserialize, Serialize};

/// Required before a client may create, settle, or read durable `/goal` records.
pub const GOAL_CAPABILITY: &str = "goal-v1";
/// Maximum encoded size of one goal endpoint frame.
pub const MAX_GOAL_FRAME_BYTES: usize = 64 * 1024;
const MAX_GOALS_PER_RESPONSE: usize = 128;

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
    Create {
        request_id: String,
        goal: GoalRecord,
    },
    Created {
        request_id: String,
        goal: GoalRecord,
    },
    Transition {
        request_id: String,
        transition: GoalTransition,
    },
    Transitioned {
        request_id: String,
        goal: GoalRecord,
    },
    Read {
        request_id: String,
        binding: GoalBinding,
    },
    Goal {
        request_id: String,
        binding: GoalBinding,
        goal: Option<GoalRecord>,
    },
    List {
        request_id: String,
        conversation_id: AgentChatConversationId,
    },
    Goals {
        request_id: String,
        conversation_id: AgentChatConversationId,
        goals: Vec<GoalRecord>,
    },
}

impl GoalFrame {
    /// Validates bounded correlation and goal values before dedicated transport.
    ///
    /// # Errors
    /// Returns an error for malformed, mismatched, or oversized public frames.
    pub fn validate(&self) -> Result<(), GoalFrameError> {
        match self {
            Self::Create { request_id, goal }
            | Self::Created { request_id, goal }
            | Self::Transitioned { request_id, goal } => {
                valid_id(request_id)?;
                goal.validate()?;
            }
            Self::Transition {
                request_id,
                transition,
            } => {
                valid_id(request_id)?;
                transition.validate()?;
            }
            Self::Read {
                request_id,
                binding,
            } => {
                valid_id(request_id)?;
                validate_binding(binding)?;
            }
            Self::Goal {
                request_id,
                binding,
                goal,
            } => {
                valid_id(request_id)?;
                validate_binding(binding)?;
                if let Some(goal) = goal {
                    goal.validate()?;
                    if goal.binding != *binding {
                        return Err(GoalFrameError::BindingMismatch);
                    }
                }
            }
            Self::List {
                request_id,
                conversation_id,
            } => {
                valid_id(request_id)?;
                valid_id(&conversation_id.0)?;
            }
            Self::Goals {
                request_id,
                conversation_id,
                goals,
            } => {
                valid_id(request_id)?;
                valid_id(&conversation_id.0)?;
                if goals.len() > MAX_GOALS_PER_RESPONSE {
                    return Err(GoalFrameError::TooManyGoals);
                }
                for goal in goals {
                    goal.validate()?;
                    if goal.binding.conversation_id != *conversation_id {
                        return Err(GoalFrameError::BindingMismatch);
                    }
                }
            }
        }
        if self.encoded_len()? > MAX_GOAL_FRAME_BYTES {
            return Err(GoalFrameError::TooLarge);
        }
        Ok(())
    }

    fn encoded_len(&self) -> Result<usize, GoalFrameError> {
        serde_json::to_vec(self)
            .map(|encoded| encoded.len())
            .map_err(|_| GoalFrameError::InvalidEncoding)
    }
}

fn validate_binding(binding: &GoalBinding) -> Result<(), GoalFrameError> {
    valid_id(&binding.goal_id)?;
    valid_id(&binding.conversation_id.0)?;
    valid_id(&binding.run_id.0)
}

fn valid_id(value: &str) -> Result<(), GoalFrameError> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        return Err(GoalFrameError::InvalidIdentifier);
    }
    Ok(())
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
    #[error("goal response contains too many records")]
    TooManyGoals,
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
