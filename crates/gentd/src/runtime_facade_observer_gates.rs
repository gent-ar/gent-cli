use gent_runtime::{
    AgentChatConversationAuthority, AgentChatPromptAuthority, AgentChatSelectionSwitchAuthority,
    AgentChatSessionAuthority, AutomationAuthority, ConversationActivityAuthority, GoalAuthority,
    OrchestrationAuthority, ReviewedPlanAuthority, RuntimeMaintenanceAuthority,
};

pub(super) fn chat_authority(enabled: bool) -> AgentChatConversationAuthority {
    if enabled {
        AgentChatConversationAuthority::Approved
    } else {
        AgentChatConversationAuthority::Observer
    }
}

pub(super) fn fork_authority(enabled: bool) -> gent_runtime::AgentChatForkAuthority {
    if enabled {
        gent_runtime::AgentChatForkAuthority::Approved
    } else {
        gent_runtime::AgentChatForkAuthority::Observer
    }
}

pub(super) fn checkpoint_authority(enabled: bool) -> gent_runtime::AgentChatCheckpointAuthority {
    if enabled {
        gent_runtime::AgentChatCheckpointAuthority::Approved
    } else {
        gent_runtime::AgentChatCheckpointAuthority::Observer
    }
}

pub(super) fn side_question_authority(
    enabled: bool,
) -> gent_runtime::AgentChatSideQuestionAuthority {
    if enabled {
        gent_runtime::AgentChatSideQuestionAuthority::Approved
    } else {
        gent_runtime::AgentChatSideQuestionAuthority::Observer
    }
}

pub(super) fn prompt_authority(enabled: bool) -> AgentChatPromptAuthority {
    if enabled {
        AgentChatPromptAuthority::Approved
    } else {
        AgentChatPromptAuthority::Observer
    }
}

pub(super) fn automation_authority(enabled: bool) -> AutomationAuthority {
    if enabled {
        AutomationAuthority::Approved
    } else {
        AutomationAuthority::Observer
    }
}

pub(super) fn session_authority(enabled: bool) -> AgentChatSessionAuthority {
    if enabled {
        AgentChatSessionAuthority::Approved
    } else {
        AgentChatSessionAuthority::Observer
    }
}

pub(super) fn switch_authority(enabled: bool) -> AgentChatSelectionSwitchAuthority {
    if enabled {
        AgentChatSelectionSwitchAuthority::Approved
    } else {
        AgentChatSelectionSwitchAuthority::Observer
    }
}

pub(super) fn activity_authority(enabled: bool) -> ConversationActivityAuthority {
    if enabled {
        ConversationActivityAuthority::Approved
    } else {
        ConversationActivityAuthority::Observer
    }
}

pub(super) fn maintenance_authority(enabled: bool) -> RuntimeMaintenanceAuthority {
    if enabled {
        RuntimeMaintenanceAuthority::Approved
    } else {
        RuntimeMaintenanceAuthority::Observer
    }
}

pub(super) fn goal_authority(enabled: bool) -> GoalAuthority {
    if enabled {
        GoalAuthority::Approved
    } else {
        GoalAuthority::Observer
    }
}

pub(super) fn orchestration_authority(enabled: bool) -> OrchestrationAuthority {
    if enabled {
        OrchestrationAuthority::Approved
    } else {
        OrchestrationAuthority::Observer
    }
}

pub(super) fn reviewed_plan_authority(enabled: bool) -> ReviewedPlanAuthority {
    if enabled {
        ReviewedPlanAuthority::Approved
    } else {
        ReviewedPlanAuthority::Observer
    }
}
