use gent_protocol::AgentChatIntentFrame;

pub(crate) fn prompt_intent(frame: &AgentChatIntentFrame) -> bool {
    matches!(
        frame,
        AgentChatIntentFrame::SendPrompt { .. }
            | AgentChatIntentFrame::QueuePrompt { .. }
            | AgentChatIntentFrame::SendPromptWithTools { .. }
            | AgentChatIntentFrame::QueuePromptWithTools { .. }
    )
}

pub(crate) fn prompt_admission_error(
    error: crate::ordinary_lifecycle_control::OrdinaryPromptAdmissionError,
) -> crate::agent_chat_intent_error::AgentChatIntentError {
    match error {
        crate::ordinary_lifecycle_control::OrdinaryPromptAdmissionError::RecoveryInProgress => {
            gent_types::AgentChatRejection::LifecycleRecoveryInProgress
        }
        crate::ordinary_lifecycle_control::OrdinaryPromptAdmissionError::ShuttingDown => {
            gent_types::AgentChatRejection::LifecycleShuttingDown
        }
    }
    .into()
}

#[cfg(test)]
mod tests {
    use gent_protocol::AgentChatIntentFrame;
    use gent_types::{AgentChatConversationId, AgentChatRequestId, ReceiptId};

    use super::{prompt_admission_error, prompt_intent};
    use crate::ordinary_lifecycle_control::OrdinaryPromptAdmissionError;

    #[test]
    fn unavailable_prompt_admission_is_a_typed_retryable_rejection() {
        assert_eq!(
            prompt_admission_error(OrdinaryPromptAdmissionError::RecoveryInProgress).code,
            "lifecycleRecoveryInProgress"
        );
        assert_eq!(
            prompt_admission_error(OrdinaryPromptAdmissionError::ShuttingDown).code,
            "lifecycleShuttingDown"
        );
    }

    #[test]
    fn tool_bearing_prompts_acquire_the_same_admission_permit() {
        let send = AgentChatIntentFrame::SendPromptWithTools {
            request_id: AgentChatRequestId("request".into()),
            receipt_id: ReceiptId("receipt".into()),
            conversation_id: AgentChatConversationId("conversation".into()),
            text: "use tools".into(),
            attachment_ids: vec![],
            tool_source_ids: vec!["mcp:docs".into()],
        };
        let AgentChatIntentFrame::SendPromptWithTools {
            request_id,
            receipt_id,
            conversation_id,
            text,
            attachment_ids,
            tool_source_ids,
        } = send.clone()
        else {
            unreachable!();
        };
        let queue = AgentChatIntentFrame::QueuePromptWithTools {
            request_id,
            receipt_id,
            conversation_id,
            text,
            attachment_ids,
            tool_source_ids,
        };
        assert!(prompt_intent(&send));
        assert!(prompt_intent(&queue));
    }
}
