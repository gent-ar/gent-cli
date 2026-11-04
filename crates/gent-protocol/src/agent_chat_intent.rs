//! Capability-gated future agent-chat intent frames.
//!
//! No runtime currently composes this endpoint. Negotiating these frames alone
//! never starts a provider, creates a conversation, or changes observer mode.

use gent_types::{
    AgentChatConversationId, AgentChatDecisionId, AgentChatDecisionResponse,
    AgentChatPromptDelivery, AgentChatRequestId, AgentChatRunId, AgentChatSelection, ContextPolicy,
    ConversationMessageDelivery, ConversationWaitResult, LinkedConversations,
    NormalizedTranscriptEvent, NormalizedTranscriptKind, Receipt,
};
use serde::{Deserialize, Serialize};

/// Required before a client may submit future agent-chat intents.
pub const AGENT_CHAT_INTENTS_CAPABILITY: &str = "agent-chat-intents-v1";
pub const AGENT_CHAT_STEER_TURN_CAPABILITY: &str = "agent-chat-steer-turn-v1";
pub const AGENT_CHAT_TRANSCRIPT_IMPORT_CAPABILITY: &str = "agent-chat-transcript-import-v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoricalTranscriptEntry {
    pub source_id: String,
    pub kind: NormalizedTranscriptKind,
    pub text: String,
}

/// Future receipt-backed commands plus cursor-based subscription requests.
///
/// Providers, credentials, and provider-native session identifiers never cross
/// this public contract. An authority runtime must still validate every intent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AgentChatIntentFrame {
    SteerPromptForTurn(crate::AgentChatSteerPrompt),
    WarmProvider(crate::AgentChatProviderWarm),
    ProviderWarm(crate::AgentChatProviderWarmResult),
    CreateConversation {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        /// Raw local path, canonicalized and validated by gentd before it is persisted.
        workspace_path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selection: Option<AgentChatSelection>,
    },
    ImportTranscript {
        request_id: AgentChatRequestId,
        conversation_id: AgentChatConversationId,
        run_id: AgentChatRunId,
        entries: Vec<HistoricalTranscriptEntry>,
    },
    SendPrompt {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        conversation_id: AgentChatConversationId,
        text: String,
        attachment_ids: Vec<String>,
    },
    QueuePrompt {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        conversation_id: AgentChatConversationId,
        text: String,
        attachment_ids: Vec<String>,
    },
    SendPromptWithTools {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        conversation_id: AgentChatConversationId,
        text: String,
        attachment_ids: Vec<String>,
        tool_source_ids: Vec<String>,
    },
    QueuePromptWithTools {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        conversation_id: AgentChatConversationId,
        text: String,
        attachment_ids: Vec<String>,
        tool_source_ids: Vec<String>,
    },
    CancelQueuedPrompt {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        conversation_id: AgentChatConversationId,
        message_id: String,
    },
    QueuedPromptCanceled {
        request_id: AgentChatRequestId,
        receipt: Receipt,
        conversation_id: AgentChatConversationId,
        message_id: String,
    },
    SteerQueuedPromptForTurn {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        conversation_id: AgentChatConversationId,
        message_id: String,
        expected_run_id: AgentChatRunId,
        expected_turn_id: String,
    },
    SteerQueuedPrompt {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        conversation_id: AgentChatConversationId,
        message_id: String,
    },
    QueuedPromptSteered {
        request_id: AgentChatRequestId,
        receipt: Receipt,
        conversation_id: AgentChatConversationId,
        message_id: String,
    },
    ContinueFromSavedHistory {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        conversation_id: AgentChatConversationId,
        message_id: String,
    },
    SwitchSelection {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        conversation_id: AgentChatConversationId,
        parent_run_id: AgentChatRunId,
        selection: AgentChatSelection,
        context_policy: ContextPolicy,
    },
    Interrupt {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        conversation_id: AgentChatConversationId,
        run_id: AgentChatRunId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_turn_id: Option<String>,
    },
    Decision {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        conversation_id: AgentChatConversationId,
        decision_id: AgentChatDecisionId,
        response: AgentChatDecisionResponse,
    },
    Subscribe {
        request_id: AgentChatRequestId,
        conversation_id: AgentChatConversationId,
        after_cursor: u64,
    },
    /// A cursor-ordered item delivered for one prior subscription request.
    SubscriptionEvent {
        request_id: AgentChatRequestId,
        event: NormalizedTranscriptEvent,
    },
    /// Explicit terminal state for a subscription; a client must resubscribe.
    SubscriptionEnded {
        request_id: AgentChatRequestId,
        reason: AgentChatSubscriptionEnd,
    },
    /// Durable result of creating a conversation and its immutable root run.
    Created {
        request_id: AgentChatRequestId,
        receipt: Receipt,
        conversation_id: AgentChatConversationId,
        run_id: AgentChatRunId,
    },
    TranscriptImported {
        request_id: AgentChatRequestId,
        conversation_id: AgentChatConversationId,
        run_id: AgentChatRunId,
        imported_count: u16,
    },
    /// Durable result of selecting a new immutable child run for one conversation.
    Switched {
        request_id: AgentChatRequestId,
        receipt: Receipt,
        conversation_id: AgentChatConversationId,
        parent_run_id: AgentChatRunId,
        run_id: AgentChatRunId,
        context_policy: ContextPolicy,
        context_through_ordinal: u64,
    },
    Accepted {
        request_id: AgentChatRequestId,
        receipt: Receipt,
        /// Canonical conversation resolved while committing the prompt.
        conversation_id: AgentChatConversationId,
        /// Immutable run selected by the ledger while committing the prompt.
        run_id: AgentChatRunId,
        /// Immutable turn created by the same transaction as the receipt.
        turn_id: String,
        message_id: String,
        /// Durable local delivery state; this never attests that a provider was launched.
        delivery: AgentChatPromptDelivery,
    },
    Interrupted {
        request_id: AgentChatRequestId,
        receipt: Receipt,
        conversation_id: AgentChatConversationId,
        run_id: AgentChatRunId,
    },
    ForkConversation {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        source_conversation_id: AgentChatConversationId,
        fork_through_message_id: String,
    },
    /// Durable result of forking a conversation's prior messages into a new one.
    Forked {
        request_id: AgentChatRequestId,
        receipt: Receipt,
        source_conversation_id: AgentChatConversationId,
        conversation_id: AgentChatConversationId,
        run_id: AgentChatRunId,
    },
    CreateLinkedConversation {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        parent_conversation_id: AgentChatConversationId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selection: Option<AgentChatSelection>,
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        origin_tool_use_id: Option<String>,
    },
    LinkedConversationCreated {
        request_id: AgentChatRequestId,
        receipt: Receipt,
        parent_conversation_id: AgentChatConversationId,
        conversation_id: AgentChatConversationId,
        run_id: AgentChatRunId,
        label: String,
    },
    SendToConversation {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        from_conversation_id: AgentChatConversationId,
        target_conversation_id: AgentChatConversationId,
        message: String,
    },
    ConversationMessageDelivered {
        request_id: AgentChatRequestId,
        receipt: Receipt,
        from_conversation_id: AgentChatConversationId,
        target_conversation_id: AgentChatConversationId,
        delivery: ConversationMessageDelivery,
        message_id: String,
    },
    WaitForConversations {
        request_id: AgentChatRequestId,
        from_conversation_id: AgentChatConversationId,
        conversation_ids: Vec<AgentChatConversationId>,
        timeout_seconds: u32,
    },
    ConversationWaitSettled {
        request_id: AgentChatRequestId,
        from_conversation_id: AgentChatConversationId,
        results: Vec<ConversationWaitResult>,
        timed_out: bool,
    },
    ListLinkedConversations {
        request_id: AgentChatRequestId,
        conversation_id: AgentChatConversationId,
    },
    ConversationLinks {
        request_id: AgentChatRequestId,
        conversation_id: AgentChatConversationId,
        links: LinkedConversations,
    },
}

/// Why a future agent-chat subscription has stopped producing events.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum AgentChatSubscriptionEnd {
    ResyncRequired,
    ServerClosing,
}

#[cfg(test)]
#[path = "agent_chat_intent_tests.rs"]
mod tests;
