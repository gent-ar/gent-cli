use gent_protocol::AgentChatIntentFrame;
use gent_protocol::sandbox_unavailable::SandboxUnavailable;
use gent_types::AgentChatConversationSummary;

use crate::agent_chat_intent_error::AgentChatIntentError;

impl super::RuntimeFacade {
    pub(super) fn require_prompt_admission(
        &self,
        conversation_id: &str,
    ) -> Result<(), AgentChatIntentError> {
        let Some(summary) = self.conversation_summary(conversation_id) else {
            return Ok(());
        };
        require_workspace(&summary)?;
        let provider = summary.selection.provider;
        crate::permission_backend_registry::require(provider, summary.containment).map_err(
            |reason| {
                AgentChatIntentError::sandbox_unavailable(
                    SandboxUnavailable {
                        provider,
                        containment: summary.containment,
                    },
                    reason,
                )
            },
        )
    }

    fn conversation_summary(&self, conversation_id: &str) -> Option<AgentChatConversationSummary> {
        let detail = self
            .agent_chat_reads
            .as_ref()?
            .detail(conversation_id)
            .ok()?;
        Some(detail.summary)
    }
}

fn require_workspace(summary: &AgentChatConversationSummary) -> Result<(), AgentChatIntentError> {
    if let Some(path) = &summary.workspace_path {
        crate::workspace_identity::require_directory(path)?;
    }
    Ok(())
}

pub(crate) fn admitted_conversation(frame: &AgentChatIntentFrame) -> Option<&str> {
    match frame {
        AgentChatIntentFrame::WarmProvider(warm) => Some(&warm.conversation_id.0),
        AgentChatIntentFrame::CreateLinkedConversation {
            parent_conversation_id,
            ..
        } => Some(&parent_conversation_id.0),
        AgentChatIntentFrame::SendPrompt {
            conversation_id, ..
        }
        | AgentChatIntentFrame::QueuePrompt {
            conversation_id, ..
        }
        | AgentChatIntentFrame::SendPromptWithTools {
            conversation_id, ..
        }
        | AgentChatIntentFrame::QueuePromptWithTools {
            conversation_id, ..
        }
        | AgentChatIntentFrame::SteerQueuedPrompt {
            conversation_id, ..
        }
        | AgentChatIntentFrame::SteerQueuedPromptForTurn {
            conversation_id, ..
        }
        | AgentChatIntentFrame::ContinueFromSavedHistory {
            conversation_id, ..
        }
        | AgentChatIntentFrame::SendToConversation {
            target_conversation_id: conversation_id,
            ..
        } => Some(&conversation_id.0),
        AgentChatIntentFrame::SteerPromptForTurn(steer) => Some(&steer.conversation_id.0),
        _ => None,
    }
}

pub(crate) fn prompt_intent(frame: &AgentChatIntentFrame) -> bool {
    matches!(
        frame,
        AgentChatIntentFrame::SendPrompt { .. }
            | AgentChatIntentFrame::SteerPromptForTurn(_)
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

    use super::{admitted_conversation, prompt_admission_error, prompt_intent};
    use crate::ordinary_lifecycle_control::OrdinaryPromptAdmissionError;

    #[test]
    fn warming_and_linked_creation_pass_the_prompt_admission_check_for_their_conversation() {
        let warm = AgentChatIntentFrame::WarmProvider(gent_protocol::AgentChatProviderWarm {
            request_id: AgentChatRequestId("warm".into()),
            conversation_id: AgentChatConversationId("warmed".into()),
            run_id: gent_types::AgentChatRunId("run".into()),
        });
        let linked = AgentChatIntentFrame::CreateLinkedConversation {
            request_id: AgentChatRequestId("linked".into()),
            receipt_id: ReceiptId("linked".into()),
            parent_conversation_id: AgentChatConversationId("parent".into()),
            workspace_path: None,
            selection: None,
            label: "child".into(),
            prompt: Some("start".into()),
            origin_tool_use_id: None,
        };
        assert_eq!(admitted_conversation(&warm), Some("warmed"));
        assert_eq!(admitted_conversation(&linked), Some("parent"));
    }

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
