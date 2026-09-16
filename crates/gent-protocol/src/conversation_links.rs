use gent_types::{
    MAX_CONVERSATION_WAIT_SECONDS, MAX_CONVERSATION_WAIT_TARGETS, valid_conversation_label,
};

use crate::AgentChatIntentFrame;

pub const CONVERSATION_LINKS_CAPABILITY: &str = "conversation-links-v1";

#[must_use]
pub fn is_conversation_link_frame(frame: &AgentChatIntentFrame) -> bool {
    matches!(
        frame,
        AgentChatIntentFrame::CreateLinkedConversation { .. }
            | AgentChatIntentFrame::SendToConversation { .. }
            | AgentChatIntentFrame::WaitForConversations { .. }
            | AgentChatIntentFrame::ListLinkedConversations { .. }
            | AgentChatIntentFrame::LinkedConversationCreated { .. }
            | AgentChatIntentFrame::ConversationMessageDelivered { .. }
            | AgentChatIntentFrame::ConversationWaitSettled { .. }
            | AgentChatIntentFrame::ConversationLinks { .. }
    )
}

pub fn validate_conversation_link_request(
    frame: &AgentChatIntentFrame,
) -> Result<(), ConversationLinkRequestError> {
    match frame {
        AgentChatIntentFrame::CreateLinkedConversation {
            parent_conversation_id,
            label,
            prompt,
            ..
        } => {
            if parent_conversation_id.0.trim().is_empty() {
                return Err(ConversationLinkRequestError::UnknownConversation);
            }
            if !valid_conversation_label(label) {
                return Err(ConversationLinkRequestError::InvalidLabel);
            }
            if prompt.as_deref().is_some_and(|text| text.trim().is_empty()) {
                return Err(ConversationLinkRequestError::EmptyMessage);
            }
            Ok(())
        }
        AgentChatIntentFrame::SendToConversation {
            from_conversation_id,
            target_conversation_id,
            message,
            ..
        } => {
            if from_conversation_id == target_conversation_id {
                return Err(ConversationLinkRequestError::SelfTargeted);
            }
            if message.trim().is_empty() {
                return Err(ConversationLinkRequestError::EmptyMessage);
            }
            Ok(())
        }
        AgentChatIntentFrame::WaitForConversations {
            from_conversation_id,
            conversation_ids,
            ..
        } => {
            if conversation_ids.is_empty() || conversation_ids.len() > MAX_CONVERSATION_WAIT_TARGETS
            {
                return Err(ConversationLinkRequestError::InvalidTargetCount);
            }
            if conversation_ids
                .iter()
                .any(|target| target == from_conversation_id)
            {
                return Err(ConversationLinkRequestError::SelfTargeted);
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

#[must_use]
pub const fn bounded_wait_seconds(requested: u32) -> u32 {
    if requested > MAX_CONVERSATION_WAIT_SECONDS {
        MAX_CONVERSATION_WAIT_SECONDS
    } else {
        requested
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ConversationLinkRequestError {
    #[error("a conversation cannot address or await itself")]
    SelfTargeted,
    #[error("a linked conversation needs a short nonempty label")]
    InvalidLabel,
    #[error("a cross-conversation message cannot be empty")]
    EmptyMessage,
    #[error("a wait must name between one and sixteen conversations")]
    InvalidTargetCount,
    #[error("the named conversation is unknown")]
    UnknownConversation,
}

impl ConversationLinkRequestError {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::SelfTargeted => "conversationSelfTargeted",
            Self::InvalidLabel => "conversationLabelInvalid",
            Self::EmptyMessage => "conversationMessageEmpty",
            Self::InvalidTargetCount => "conversationWaitTargetsInvalid",
            Self::UnknownConversation => "conversationNotFound",
        }
    }
}

#[cfg(test)]
#[path = "conversation_links_tests.rs"]
mod tests;
