use gent_protocol::AgentChatIntentFrame;

pub(crate) fn prompt_intent(frame: &AgentChatIntentFrame) -> bool {
    matches!(
        frame,
        AgentChatIntentFrame::SendPrompt { .. } | AgentChatIntentFrame::QueuePrompt { .. }
    )
}

pub(crate) fn prompt_admission_error(
    error: crate::ordinary_lifecycle_control::OrdinaryPromptAdmissionError,
) -> String {
    match error {
        crate::ordinary_lifecycle_control::OrdinaryPromptAdmissionError::RecoveryInProgress => {
            "ordinary lifecycle recovery is still in progress".to_owned()
        }
        crate::ordinary_lifecycle_control::OrdinaryPromptAdmissionError::ShuttingDown => {
            "ordinary lifecycle is shutting down".to_owned()
        }
    }
}
