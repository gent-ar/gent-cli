use serde::{Deserialize, Serialize};

use crate::{
    AgentChatSelection, AgentChatSideQuestionFailureCode, AgentChatSideQuestionStatus,
    ConversationMessageDelivery, ConversationWaitTarget, GoalRecord, HostEpoch, PlanArtifact,
    PromptHoldReason, RootActivity, TokenUsage, ToolActivity, TurnPhase, WorkPhase,
};

pub const CONVERSATION_ACTIVITY_SCHEMA_VERSION: u16 = 1;

#[path = "conversation_activity_scope.rs"]
mod scope;
#[path = "conversation_activity_turn.rs"]
mod turn;
pub use turn::{ContextClearCommand, TurnTerminalCause, WorkspaceFileChange};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityWorkKind {
    Command,
    Subagent,
}

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
pub enum ModelDownloadPhase {
    Downloading,
    Verifying,
    Ready,
    Failed,
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
    ContextCompacted {
        #[serde(flatten)]
        scope: ConversationActivityScope,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request: Option<Box<crate::PermissionDecisionRequest>>,
    },
    DecisionSettled {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        #[serde(alias = "decision_id")]
        decision_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outcome: Option<crate::DecisionOutcome>,
    },
    PromptQueued {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        message_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        queue_delivery: Option<crate::AgentChatQueueDelivery>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cancelable: Option<bool>,
    },
    PromptHeld {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        message_id: String,
        receipt_id: String,
        reason: PromptHoldReason,
    },
    ModelDownloadProgress {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        model_id: String,
        message_id: String,
        receipt_id: String,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        phase: ModelDownloadPhase,
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
    PromptCancelRequested {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        message_id: String,
        receipt_id: String,
    },
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
    PromptWithdrawn {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        message_id: String,
        receipt_id: String,
        text: String,
        attachments: Vec<crate::AttachmentReference>,
    },
    InterruptRequested {
        #[serde(flatten)]
        scope: ConversationActivityScope,
    },
    ContextCleared {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<ContextClearCommand>,
    },
    WorkspaceChanges {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        files: Vec<WorkspaceFileChange>,
        truncated: bool,
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
    SideQuestionSettled {
        #[serde(flatten)]
        scope: ConversationActivityScope,
        side_question_id: String,
        ask_receipt_id: String,
        status: AgentChatSideQuestionStatus,
        answer: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure_code: Option<AgentChatSideQuestionFailureCode>,
        failure_reason: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationActivityPage {
    pub facts: Vec<ConversationActivityFact>,
    pub next_after_cursor: Option<u64>,
}

#[cfg(test)]
#[path = "conversation_activity_tests.rs"]
mod tests;
