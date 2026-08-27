use gent_types::{AgentChatPromptDelivery, ConversationListItem};

use crate::terminal;

pub(super) fn result(
    conversation_id: String,
    parent_run_id: Option<String>,
    notice: impl Into<String>,
) -> terminal::UiRequestResult {
    terminal::UiRequestResult {
        conversation: ConversationListItem {
            conversation_id,
            run_count: 1,
        },
        parent_run_id,
        notice: notice.into(),
        permission_mode: None,
        session: None,
        awaiting_turn: None,
    }
}

pub(crate) const fn delivery_notice(delivery: AgentChatPromptDelivery) -> &'static str {
    match delivery {
        AgentChatPromptDelivery::Queued => {
            "Prompt queued. Gent will continue when the provider is available."
        }
        AgentChatPromptDelivery::AwaitingReadiness => "Gent is preparing the selected provider…",
        AgentChatPromptDelivery::AwaitingProvider => "Gent is thinking…",
    }
}
