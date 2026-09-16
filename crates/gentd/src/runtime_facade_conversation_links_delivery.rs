use std::time::{Duration, Instant};

use gent_ports::ConversationLinkLedger;
use gent_protocol::{AgentChatIntentFrame, conversation_links::bounded_wait_seconds};
use gent_types::{
    AgentChatConversationId, AgentChatRequestId, ConversationActivityFact,
    ConversationMessageDelivery, ConversationWaitResult, ConversationWaitTarget,
    MAX_CONVERSATION_MESSAGE_PREVIEW_BYTES, MAX_CONVERSATION_WAIT_REPLY_BYTES, ReceiptId,
    bounded_excerpt,
};

use super::{RuntimeFacade, require_same_workspace, scope_for};
use crate::{agent_chat_intent_error::AgentChatIntentError, api::RuntimeApi};

const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(250);

impl RuntimeFacade {
    pub(super) fn send_to_conversation(
        &self,
        request_id: &AgentChatRequestId,
        receipt_id: &ReceiptId,
        from_conversation_id: &AgentChatConversationId,
        target_conversation_id: &AgentChatConversationId,
        message: &str,
    ) -> Result<Vec<AgentChatIntentFrame>, AgentChatIntentError> {
        let from = self.thread_state(from_conversation_id)?;
        let target = self.thread_state(target_conversation_id)?;
        require_same_workspace(&from, &target)?;
        let delivery = if target.is_running() {
            ConversationMessageDelivery::Steered
        } else {
            ConversationMessageDelivery::Queued
        };
        let accepted = self.agent_chat_intent(match delivery {
            ConversationMessageDelivery::Queued => AgentChatIntentFrame::SendPrompt {
                request_id: AgentChatRequestId(format!("linked-send-{}", request_id.0)),
                receipt_id: receipt_id.clone(),
                conversation_id: target_conversation_id.clone(),
                text: message.to_owned(),
                attachment_ids: Vec::new(),
            },
            ConversationMessageDelivery::Steered => AgentChatIntentFrame::QueuePrompt {
                request_id: AgentChatRequestId(format!("linked-send-{}", request_id.0)),
                receipt_id: receipt_id.clone(),
                conversation_id: target_conversation_id.clone(),
                text: message.to_owned(),
                attachment_ids: Vec::new(),
            },
        })?;
        let Some(AgentChatIntentFrame::Accepted {
            receipt,
            message_id,
            ..
        }) = accepted.into_iter().next()
        else {
            return Err("cross-conversation message was not accepted".into());
        };
        if delivery == ConversationMessageDelivery::Steered {
            self.agent_chat_intent(AgentChatIntentFrame::SteerQueuedPrompt {
                request_id: AgentChatRequestId(format!("linked-steer-{}", request_id.0)),
                receipt_id: ReceiptId(format!("linked-steer-receipt-{}", request_id.0)),
                conversation_id: target_conversation_id.clone(),
                message_id: message_id.clone(),
            })?;
        }
        let host_epoch = self.host_epoch()?;
        self.conversation_links()?
            .append_conversation_orchestration_fact(
                &ConversationActivityFact::ConversationMessageSent {
                    scope: scope_for(&from, host_epoch),
                    target_conversation_id: target_conversation_id.0.clone(),
                    target_label: self.conversation_label(&target_conversation_id.0),
                    delivery,
                    preview: bounded_excerpt(message, MAX_CONVERSATION_MESSAGE_PREVIEW_BYTES),
                },
                &format!("send:{}", request_id.0),
            )
            .map_err(|error| AgentChatIntentError::from(error.to_string()))?;
        Ok(vec![AgentChatIntentFrame::ConversationMessageDelivered {
            request_id: request_id.clone(),
            receipt,
            from_conversation_id: from_conversation_id.clone(),
            target_conversation_id: target_conversation_id.clone(),
            delivery,
            message_id,
        }])
    }

    pub(super) fn wait_for_conversations(
        &self,
        request_id: &AgentChatRequestId,
        from_conversation_id: &AgentChatConversationId,
        conversation_ids: &[AgentChatConversationId],
        timeout_seconds: u32,
    ) -> Result<Vec<AgentChatIntentFrame>, AgentChatIntentError> {
        let from = self.thread_state(from_conversation_id)?;
        for conversation_id in conversation_ids {
            let target = self.thread_state(conversation_id)?;
            require_same_workspace(&from, &target)?;
        }
        let deadline =
            Instant::now() + Duration::from_secs(u64::from(bounded_wait_seconds(timeout_seconds)));
        let mut results = self.probe(conversation_ids)?;
        while !results.iter().all(|result| result.target.is_settled()) {
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(WAIT_POLL_INTERVAL);
            results = self.probe(conversation_ids)?;
        }
        let timed_out = !results.iter().all(|result| result.target.is_settled());
        let host_epoch = self.host_epoch()?;
        self.conversation_links()?
            .append_conversation_orchestration_fact(
                &ConversationActivityFact::ConversationWaitSettled {
                    scope: scope_for(&from, host_epoch),
                    targets: results
                        .iter()
                        .map(|result| result.target.clone())
                        .collect::<Vec<_>>(),
                    timed_out,
                },
                &format!("wait:{}", request_id.0),
            )
            .map_err(|error| AgentChatIntentError::from(error.to_string()))?;
        Ok(vec![AgentChatIntentFrame::ConversationWaitSettled {
            request_id: request_id.clone(),
            from_conversation_id: from_conversation_id.clone(),
            results,
            timed_out,
        }])
    }

    fn probe(
        &self,
        conversation_ids: &[AgentChatConversationId],
    ) -> Result<Vec<ConversationWaitResult>, AgentChatIntentError> {
        conversation_ids
            .iter()
            .map(|conversation_id| {
                let state = self.thread_state(conversation_id)?;
                let label = self.conversation_label(&state.conversation_id);
                Ok(ConversationWaitResult {
                    target: ConversationWaitTarget {
                        conversation_id: state.conversation_id,
                        label,
                        phase: state.phase,
                    },
                    reply: state
                        .last_assistant_text
                        .map(|text| bounded_excerpt(&text, MAX_CONVERSATION_WAIT_REPLY_BYTES)),
                })
            })
            .collect()
    }
}
