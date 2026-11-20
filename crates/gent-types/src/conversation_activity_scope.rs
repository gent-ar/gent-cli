use super::{ConversationActivityFact, ConversationActivityScope};

impl ConversationActivityFact {
    #[must_use]
    pub fn scope(&self) -> &ConversationActivityScope {
        match self {
            Self::TurnStarted { scope }
            | Self::ContextUsage { scope, .. }
            | Self::ContextCompacted { scope }
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
            | Self::ModelDownloadProgress { scope, .. }
            | Self::PromptReleased { scope, .. }
            | Self::PromptCanceled { scope, .. }
            | Self::PromptCancelRequested { scope, .. }
            | Self::PromptSteerRequested { scope, .. }
            | Self::PromptSteerRejected { scope, .. }
            | Self::PromptSteered { scope, .. }
            | Self::PromptWithdrawn { scope, .. }
            | Self::InterruptRequested { scope }
            | Self::ContextCleared { scope, .. }
            | Self::WorkspaceChanges { scope, .. }
            | Self::Recovered { scope }
            | Self::Terminal { scope, .. }
            | Self::GoalUpdated { scope, .. }
            | Self::PlanUpdated { scope, .. }
            | Self::ConversationCreated { scope, .. }
            | Self::CreatedByConversation { scope, .. }
            | Self::ConversationMessageSent { scope, .. }
            | Self::ConversationWaitSettled { scope, .. }
            | Self::SideQuestionSettled { scope, .. } => scope,
        }
    }
}
