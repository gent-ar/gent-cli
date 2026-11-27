#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AgentChatRejection {
    #[error("the planning turn must settle before its plan can be approved")]
    PlanApprovalBlockedByActiveTurn,
    #[error("this plan revision is no longer awaiting review")]
    PlanNotReviewable,
    #[error("the plan can be approved only from the conversation's current run")]
    PlanApprovalParentNotCurrent,
    #[error("the workspace permission policy changed after the plan was shown")]
    PlanApprovalPolicyChanged,
    #[error("the selection can change only from the conversation's current run")]
    SelectionSwitchParentNotCurrent,
    #[error("the selection cannot change while provider installation is reserved for a prompt")]
    SelectionSwitchBlockedByProvisioning,
    #[error("the prompt is no longer queued, or its provider has already taken it")]
    QueuedPromptNotCancelable,
    #[error("the prompt is no longer queued for delivery into this conversation")]
    QueuedPromptNotSteerable,
    #[error("Gentd is still recovering provider lifecycles; retry the prompt shortly")]
    LifecycleRecoveryInProgress,
    #[error("Gentd is shutting down and no longer accepts prompts")]
    LifecycleShuttingDown,
    #[error("the conversation does not exist")]
    ConversationNotFound,
    #[error("the conversation is archived and accepts no new prompts")]
    ConversationArchived,
    #[error("the selected model is not offered by this provider")]
    SelectionModelUnavailable,
    #[error("the selected model does not offer this effort")]
    SelectionEffortUnavailable,
    #[error("nothing is running in this conversation to interrupt")]
    NothingToInterrupt,
}

impl AgentChatRejection {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::PlanApprovalBlockedByActiveTurn => "planApprovalBlockedByActiveTurn",
            Self::PlanNotReviewable => "planNotReviewable",
            Self::PlanApprovalParentNotCurrent => "planApprovalParentNotCurrent",
            Self::PlanApprovalPolicyChanged => "planApprovalPolicyChanged",
            Self::SelectionSwitchParentNotCurrent => "selectionSwitchParentNotCurrent",
            Self::SelectionSwitchBlockedByProvisioning => "selectionSwitchBlockedByProvisioning",
            Self::QueuedPromptNotCancelable => "queuedPromptNotCancelable",
            Self::QueuedPromptNotSteerable => "queuedPromptNotSteerable",
            Self::LifecycleRecoveryInProgress => "lifecycleRecoveryInProgress",
            Self::LifecycleShuttingDown => "lifecycleShuttingDown",
            Self::ConversationNotFound => "conversationNotFound",
            Self::ConversationArchived => "conversationArchived",
            Self::SelectionModelUnavailable => "selectionModelUnavailable",
            Self::SelectionEffortUnavailable => "selectionEffortUnavailable",
            Self::NothingToInterrupt => "nothingToInterrupt",
        }
    }
}
