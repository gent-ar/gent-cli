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

/// The bounded effort choices which a compatible agent-chat client may render.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum AgentChatEffort {
    Low,
    Medium,
    High,
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
    pub runs: Vec<AgentChatRun>,
}

/// A normalized kind of transcript entry. It is never a raw provider event.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum NormalizedTranscriptKind {
    UserMessage,
    AssistantMessage,
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
        AgentChatSelection, NormalizedTranscriptEvent, NormalizedTranscriptKind,
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
}
