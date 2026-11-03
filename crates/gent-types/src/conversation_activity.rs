use serde::{Deserialize, Serialize};

use crate::{
    AgentChatSelection, ConversationMessageDelivery, ConversationWaitTarget, GoalRecord, HostEpoch,
    PlanArtifact, PromptHoldReason, RootActivity, TokenUsage, ToolActivity, TurnPhase, WorkPhase,
};

/// Version of the conversation-activity value contract.
pub const CONVERSATION_ACTIVITY_SCHEMA_VERSION: u16 = 1;

/// The kind of work described by an activity fact.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityWorkKind {
    Command,
    Subagent,
}

/// Identity and ordering data every activity fact must carry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationActivityScope {
    pub conversation_id: String,
    pub run_id: String,
    pub turn_id: String,
    pub host_epoch: HostEpoch,
    pub cursor: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TurnTerminalCause {
    Steered,
    DeliveryUnprovable,
    ProviderSessionUnavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ConversationActivityFact {
    TurnStarted {
        #[serde(flatten)]
        scope: ConversationActivityScope,
    },
    ContextUsage {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        #[serde(alias = "used_tokens")]
        used_tokens: u64,
        #[serde(alias = "window_tokens")]
        window_tokens: Option<u64>,
    },
    TokenUsage {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        usage: TokenUsage,
    },
    RootActivity {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        activity: RootActivity,
    },
    RootPhase {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        phase: TurnPhase,
    },
    WorkPhase {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        #[serde(alias = "work_id")]
        work_id: String,
        kind: ActivityWorkKind,
        phase: WorkPhase,
    },
    ToolActivity {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        activity: ToolActivity,
    },
    ToolOutput {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        tool_use_id: String,
        text: String,
        is_partial: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_lifecycle_event_id: Option<String>,
    },
    SubagentStarted {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        #[serde(alias = "child_id")]
        child_id: String,
        #[serde(alias = "parent_tool_use_id")]
        parent_tool_use_id: String,
    },
    DecisionPending {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        #[serde(alias = "decision_id")]
        decision_id: String,
    },
    DecisionSettled {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        #[serde(alias = "decision_id")]
        decision_id: String,
    },
    PromptQueued {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        message_id: String,
    },
    PromptHeld {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        message_id: String,
        receipt_id: String,
        reason: PromptHoldReason,
    },
    PromptReleased {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        message_id: String,
    },
    PromptCanceled {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        message_id: String,
    },
    /// Durable request to steer; provider consumption is recorded separately.
    PromptSteerRequested {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        message_id: String,
        receipt_id: String,
    },
    PromptSteerRejected {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        message_id: String,
        receipt_id: String,
        reason: String,
    },
    PromptSteered {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        message_id: String,
        receipt_id: String,
        transcript_cursor: u64,
    },
    InterruptRequested {
        #[serde(flatten)]
        scope: ConversationActivityScope,
    },
    Recovered {
        #[serde(flatten)]
        scope: ConversationActivityScope,
    },
    Terminal {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        phase: TurnPhase,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cause: Option<TurnTerminalCause>,
    },
    GoalUpdated {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        goal: GoalRecord,
    },
    PlanUpdated {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        plan: PlanArtifact,
    },
    ConversationCreated {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        child_conversation_id: String,
        label: String,
        workspace_path: String,
        selection: AgentChatSelection,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        origin_tool_use_id: Option<String>,
    },
    CreatedByConversation {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        parent_conversation_id: String,
        parent_run_id: String,
        label: String,
    },
    ConversationMessageSent {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        target_conversation_id: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        target_label: String,
        delivery: ConversationMessageDelivery,
        preview: String,
    },
    ConversationWaitSettled {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        targets: Vec<ConversationWaitTarget>,
        timed_out: bool,
    },
}

impl ConversationActivityFact {
    /// Returns the common immutable scope carried by this fact.
    #[must_use]
    pub fn scope(&self) -> &ConversationActivityScope {
        match self {
            Self::TurnStarted { scope }
            | Self::ContextUsage { scope, .. }
            | Self::TokenUsage { scope, .. }
            | Self::RootActivity { scope, .. }
            | Self::RootPhase { scope, .. }
            | Self::WorkPhase { scope, .. }
            | Self::ToolActivity { scope, .. }
            | Self::ToolOutput { scope, .. }
            | Self::SubagentStarted { scope, .. }
            | Self::DecisionPending { scope, .. }
            | Self::DecisionSettled { scope, .. }
            | Self::PromptQueued { scope, .. }
            | Self::PromptHeld { scope, .. }
            | Self::PromptReleased { scope, .. }
            | Self::PromptCanceled { scope, .. }
            | Self::PromptSteerRequested { scope, .. }
            | Self::PromptSteerRejected { scope, .. }
            | Self::PromptSteered { scope, .. }
            | Self::InterruptRequested { scope }
            | Self::Recovered { scope }
            | Self::Terminal { scope, .. }
            | Self::GoalUpdated { scope, .. }
            | Self::PlanUpdated { scope, .. }
            | Self::ConversationCreated { scope, .. }
            | Self::CreatedByConversation { scope, .. }
            | Self::ConversationMessageSent { scope, .. }
            | Self::ConversationWaitSettled { scope, .. } => scope,
        }
    }
}

/// Bounded, cursor-ordered durable activity facts for one conversation run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationActivityPage {
    pub facts: Vec<ConversationActivityFact>,
    pub next_after_cursor: Option<u64>,
}

#[cfg(test)]
#[path = "conversation_activity_tests.rs"]
mod tests;
