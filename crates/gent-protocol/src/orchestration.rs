//! Reserved, capability-gated IPC for Gent-owned task-graph orchestration.
//!
//! The frames are deliberately provider-neutral. They do not carry commands, sessions,
//! credentials, plans, or provider output. A hard-observer daemon must not advertise this
//! capability and rejects every request before any durable or process-side effect.

use gent_types::{AgentChatConversationId, CrossReviewRequest, FanoutRequest, TaskGraph};
use serde::{Deserialize, Serialize};

/// Required before clients may read or mutate a Gent-owned orchestration graph.
pub const ORCHESTRATION_CAPABILITY: &str = "orchestration-v1";
/// Maximum encoded size of one orchestration endpoint frame.
pub const MAX_ORCHESTRATION_FRAME_BYTES: usize = 64 * 1024;

/// One finite, correlated task-graph exchange on a dedicated local endpoint.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum OrchestrationFrame {
    /// Creates the initial declarative graph; future authority validates every fence durably.
    Fanout {
        request_id: String,
        request: FanoutRequest,
    },
    /// Appends a cross-provider reviewer node to a previously durable graph.
    CrossReview {
        request_id: String,
        request: CrossReviewRequest,
    },
    /// Reads a graph scope without exposing provider-local run state.
    GraphRead {
        request_id: String,
        conversation_id: AgentChatConversationId,
        graph_id: String,
    },
    /// Correlated durable graph result for either graph-mutating request.
    GraphSaved {
        request_id: String,
        graph: TaskGraph,
    },
    /// Correlated graph lookup result; absence is explicit and terminal.
    Graph {
        request_id: String,
        conversation_id: AgentChatConversationId,
        graph_id: String,
        graph: Option<TaskGraph>,
    },
}

impl OrchestrationFrame {
    /// Validates bounded public values before dedicated local transport.
    ///
    /// # Errors
    /// Returns a value-free error for malformed, inconsistent, or oversized frames.
    pub fn validate(&self) -> Result<(), OrchestrationFrameError> {
        match self {
            Self::Fanout {
                request_id,
                request,
            } => {
                valid_id(request_id)?;
                request.validate()?;
            }
            Self::CrossReview {
                request_id,
                request,
            } => {
                valid_id(request_id)?;
                request.validate()?;
            }
            Self::GraphRead {
                request_id,
                conversation_id,
                graph_id,
            } => valid_scope(request_id, conversation_id, graph_id)?,
            Self::GraphSaved { request_id, graph } => {
                valid_id(request_id)?;
                graph.validate()?;
            }
            Self::Graph {
                request_id,
                conversation_id,
                graph_id,
                graph,
            } => {
                valid_scope(request_id, conversation_id, graph_id)?;
                if let Some(graph) = graph {
                    graph.validate()?;
                    if graph.binding.conversation_id != *conversation_id
                        || graph.binding.graph_id != *graph_id
                    {
                        return Err(OrchestrationFrameError::GraphScopeMismatch);
                    }
                }
            }
        }
        if self.encoded_len()? > MAX_ORCHESTRATION_FRAME_BYTES {
            return Err(OrchestrationFrameError::TooLarge);
        }
        Ok(())
    }

    fn encoded_len(&self) -> Result<usize, OrchestrationFrameError> {
        serde_json::to_vec(self)
            .map(|encoded| encoded.len())
            .map_err(|_| OrchestrationFrameError::InvalidEncoding)
    }
}

fn valid_scope(
    request_id: &str,
    conversation_id: &AgentChatConversationId,
    graph_id: &str,
) -> Result<(), OrchestrationFrameError> {
    valid_id(request_id)?;
    valid_id(&conversation_id.0)?;
    valid_id(graph_id)
}

fn valid_id(value: &str) -> Result<(), OrchestrationFrameError> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        return Err(OrchestrationFrameError::InvalidIdentifier);
    }
    Ok(())
}

/// Deliberately value-free validation errors for the orchestration endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum OrchestrationFrameError {
    #[error("orchestration correlation identifier is invalid")]
    InvalidIdentifier,
    #[error("orchestration request is invalid")]
    InvalidRequest,
    #[error("orchestration graph does not match its response scope")]
    GraphScopeMismatch,
    #[error("orchestration frame exceeds byte budget")]
    TooLarge,
    #[error("orchestration frame could not be encoded")]
    InvalidEncoding,
}

impl From<gent_types::OrchestrationContractError> for OrchestrationFrameError {
    fn from(_: gent_types::OrchestrationContractError) -> Self {
        Self::InvalidRequest
    }
}

#[cfg(test)]
#[path = "orchestration_tests.rs"]
mod tests;
