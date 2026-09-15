use gent_ports::{
    AttachmentLedger, ConversationActivityLedger, ConversationPromptLedger,
    MAX_CONVERSATION_ACTIVITY_PAGE_FACTS,
};
use gent_protocol::AgentChatIntentFrame;
use gent_types::{
    AgentChatRunId, ContextPolicy, ConversationActivityFact, ConversationMessage, ReceiptId,
    TurnTerminalCause,
};

use super::RuntimeFacade;
use crate::agent_chat_intent_error::AgentChatIntentError;

impl RuntimeFacade {
    pub(super) fn session_continuation_intent(
        &self,
        frame: &AgentChatIntentFrame,
    ) -> Result<Option<Vec<AgentChatIntentFrame>>, AgentChatIntentError> {
        let AgentChatIntentFrame::ContinueFromSavedHistory {
            request_id,
            receipt_id,
            conversation_id,
            message_id,
        } = frame
        else {
            return Ok(None);
        };
        let message = self
            .transcript_import_ledger
            .find_conversation_message(message_id)
            .map_err(|error| error.to_string())?
            .filter(|message| message.conversation_id == conversation_id.0)
            .ok_or("the message to continue is not part of this conversation")?;
        if !self.provider_session_unavailable(&message)? {
            return Err(
                "only a turn whose provider session is unavailable can continue from saved history"
                    .into(),
            );
        }
        let reads = self
            .agent_chat_reads
            .as_ref()
            .ok_or("agent-chat reads are unavailable")?;
        if reads.detail(&conversation_id.0)?.current_run_id == message.run_id {
            self.exchange_agent_chat_intent(AgentChatIntentFrame::SwitchSelection {
                request_id: gent_types::AgentChatRequestId(format!("{}:switch", request_id.0)),
                receipt_id: ReceiptId(format!("{}:switch", receipt_id.0)),
                conversation_id: conversation_id.clone(),
                parent_run_id: AgentChatRunId(message.run_id.clone()),
                selection: reads.run_selection(&conversation_id.0, &message.run_id)?,
                context_policy: ContextPolicy::Preserve,
            })?;
        }
        let attachment_ids = self
            .transcript_import_ledger
            .turn_attachments(&message.turn_id)
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|attachment| attachment.attachment_id)
            .collect();
        self.exchange_agent_chat_intent(AgentChatIntentFrame::SendPrompt {
            request_id: request_id.clone(),
            receipt_id: receipt_id.clone(),
            conversation_id: conversation_id.clone(),
            text: message.text,
            attachment_ids,
        })
        .map(Some)
    }

    fn provider_session_unavailable(
        &self,
        message: &ConversationMessage,
    ) -> Result<bool, AgentChatIntentError> {
        let mut after = 0;
        loop {
            let page = self
                .transcript_import_ledger
                .read_conversation_activity_page(
                    &message.conversation_id,
                    &message.run_id,
                    after,
                    MAX_CONVERSATION_ACTIVITY_PAGE_FACTS,
                )
                .map_err(|error| error.to_string())?;
            if page.facts.iter().any(|fact| {
                matches!(
                    fact,
                    ConversationActivityFact::Terminal {
                        scope,
                        cause: Some(TurnTerminalCause::ProviderSessionUnavailable),
                        ..
                    } if scope.turn_id == message.turn_id
                )
            }) {
                return Ok(true);
            }
            match page.next_after_cursor {
                Some(next) if next > after => after = next,
                _ => return Ok(false),
            }
        }
    }
}

#[cfg(test)]
#[path = "runtime_facade_session_continuation_tests.rs"]
mod tests;
