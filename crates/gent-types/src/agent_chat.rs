//! Public, provider-neutral read models for a future agent-chat client.
//!
//! These values intentionally exclude provider-native session identifiers,
//! credentials, endpoint configuration, and unnormalized provider payloads.

use serde::{Deserialize, Serialize};

/// A public provider choice supported by the agent-chat surface.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum AgentChatProvider {
    Claude,
    Codex,
    Claurst,
}

/// The user-visible model and execution preferences chosen for a conversation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentChatSelection {
    pub provider: AgentChatProvider,
    pub model: String,
    pub effort: AgentChatEffort,
    pub mode: AgentChatMode,
}

/// Largest accepted UTF-8 model identifier at the public Gent boundary.
pub(crate) const MAX_AGENT_CHAT_MODEL_BYTES: usize = 512;

/// A selection field failed the provider-neutral durable contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AgentChatSelectionError {
    /// A model identifier cannot be empty, contain a NUL, or exceed the fixed bound.
    #[error("the agent-chat model identifier is invalid")]
    InvalidModel,
}

impl AgentChatSelection {
    /// Validates only provider-neutral durable selection fields.
    ///
    /// Provider-specific model catalogs and native flags stay at their private adapter boundary.
    ///
    /// # Errors
    /// Returns an error for an empty, NUL-containing, or overlong model identifier.
    pub fn validate(&self) -> Result<(), AgentChatSelectionError> {
        let model = self.model.trim();
        if model.is_empty()
            || self.model.len() > MAX_AGENT_CHAT_MODEL_BYTES
            || self.model.contains('\0')
        {
            return Err(AgentChatSelectionError::InvalidModel);
        }
        Ok(())
    }
}

/// The bounded effort choices which a compatible agent-chat client may render.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum AgentChatEffort {
    Low,
    Medium,
    High,
    XHigh,
    Max,
    Ultra,
}

/// The bounded interaction modes which a compatible agent-chat client may render.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum AgentChatMode {
    Ask,
    Plan,
    Agent,
}

/// Content-light metadata used to render a conversation list.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentChatConversationSummary {
    pub conversation_id: String,
    pub title: Option<String>,
    #[serde(default)]
    pub recap: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub workspace_path: Option<String>,
    #[serde(default)]
    pub mcp_server_count: u16,
    #[serde(default)]
    pub mcp_server_names: Vec<String>,
    #[serde(default)]
    pub changed_file_count: Option<u32>,
    #[serde(default)]
    pub git_branch: Option<String>,
    pub updated_at_unix_ms: u64,
    pub selection: AgentChatSelection,
}

/// A run visible to a chat client without exposing a provider-native session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentChatRun {
    pub run_id: String,
    pub parent_run_id: Option<String>,
    pub selection: AgentChatSelection,
    pub state: AgentChatRunState,
}

/// A normalized run state for an agent-chat client.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum AgentChatRunState {
    Idle,
    Running,
    WaitingForUser,
    Completed,
    Interrupted,
    Failed,
}

/// The complete read model for one selected conversation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentChatConversationDetail {
    pub summary: AgentChatConversationSummary,
    /// The one durable run currently selected for future prompts.
    pub current_run_id: String,
    pub runs: Vec<AgentChatRun>,
}

/// A normalized kind of transcript entry. It is never a raw provider event.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum NormalizedTranscriptKind {
    UserMessage,
    AssistantMessage,
    Thinking,
    ToolActivity,
    Notice,
}

/// One ordered, provider-neutral event in a readable conversation transcript.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NormalizedTranscriptEvent {
    pub cursor: u64,
    pub event_id: String,
    pub turn_id: String,
    pub run_id: String,
    pub kind: NormalizedTranscriptKind,
    pub text: String,
    pub is_partial: bool,
}

/// Provider-normalized transcript content before the durable ledger assigns its cursor.
///
/// A producer supplies a stable event identity; clients never choose the cursor.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NormalizedTranscriptAppend {
    pub event_id: String,
    pub turn_id: String,
    pub run_id: String,
    pub kind: NormalizedTranscriptKind,
    pub text: String,
    pub is_partial: bool,
}

/// A cursor-paginated, ordered page of normalized transcript events.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NormalizedTranscriptPage {
    pub conversation_id: String,
    pub events: Vec<NormalizedTranscriptEvent>,
    pub next_after_cursor: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::{
        AgentChatConversationSummary, AgentChatEffort, AgentChatMode, AgentChatProvider,
        AgentChatSelection, AgentChatSelectionError, MAX_AGENT_CHAT_MODEL_BYTES,
        NormalizedTranscriptAppend, NormalizedTranscriptEvent, NormalizedTranscriptKind,
    };
    use serde_json::json;

    #[test]
    fn selection_serializes_with_stable_public_names() {
        let selection = AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "gpt-5.6".into(),
            effort: AgentChatEffort::High,
            mode: AgentChatMode::Plan,
        };
        assert_eq!(
            serde_json::to_value(selection).unwrap(),
            json!({ "provider": "codex", "model": "gpt-5.6", "effort": "high", "mode": "plan" })
        );
    }

    #[test]
    fn selection_model_is_bounded_without_constraining_provider_catalogs() {
        let selection = AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "gpt-5.6".into(),
            effort: AgentChatEffort::Medium,
            mode: AgentChatMode::Plan,
        };
        assert!(selection.validate().is_ok());
        assert_eq!(
            AgentChatSelection {
                model: " \t".into(),
                ..selection.clone()
            }
            .validate(),
            Err(AgentChatSelectionError::InvalidModel)
        );
        assert_eq!(
            AgentChatSelection {
                model: "m".repeat(MAX_AGENT_CHAT_MODEL_BYTES + 1),
                ..selection
            }
            .validate(),
            Err(AgentChatSelectionError::InvalidModel)
        );
    }

    #[test]
    fn public_read_models_reject_unknown_fields() {
        let summary = json!({
            "conversationId": "c1", "title": "A chat", "updatedAtUnixMs": 1,
            "selection": { "provider": "claude", "model": "haiku", "effort": "low", "mode": "ask" },
            "providerSessionId": "must-not-cross-the-contract"
        });
        assert!(serde_json::from_value::<AgentChatConversationSummary>(summary).is_err());

        let event = json!({
            "cursor": 1, "eventId": "e1", "turnId": "t1", "runId": "r1",
            "kind": "assistantMessage", "text": "hello", "isPartial": false,
            "credentials": "must-not-cross-the-contract"
        });
        assert!(serde_json::from_value::<NormalizedTranscriptEvent>(event).is_err());
    }

    #[test]
    fn normalized_event_round_trips_without_provider_payload() {
        let event = NormalizedTranscriptEvent {
            cursor: 1,
            event_id: "event-1".into(),
            turn_id: "turn-1".into(),
            run_id: "run-1".into(),
            kind: NormalizedTranscriptKind::AssistantMessage,
            text: "Hello".into(),
            is_partial: false,
        };
        assert_eq!(
            serde_json::from_value::<NormalizedTranscriptEvent>(
                serde_json::to_value(&event).unwrap()
            )
            .unwrap(),
            event
        );
    }

    #[test]
    fn thinking_transcript_kind_has_a_stable_public_wire_name() {
        assert_eq!(
            serde_json::to_value(NormalizedTranscriptKind::Thinking).unwrap(),
            json!("thinking")
        );
        assert_eq!(
            serde_json::from_value::<NormalizedTranscriptKind>(json!("thinking")).unwrap(),
            NormalizedTranscriptKind::Thinking
        );
    }

    #[test]
    fn append_value_cannot_claim_a_durable_cursor() {
        let append = serde_json::json!({
            "eventId": "event-1", "turnId": "turn-1", "runId": "run-1",
            "kind": "assistantMessage", "text": "Hello", "isPartial": false
        });
        assert!(serde_json::from_value::<NormalizedTranscriptAppend>(append).is_ok());
    }
}
