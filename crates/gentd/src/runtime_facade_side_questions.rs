use gent_protocol::AgentChatSideQuestionFrame;

use super::RuntimeFacade;
use crate::agent_chat_intent_error::AgentChatIntentError;

const SIDE_QUESTION_REJECTED: &str = "agentChatSideQuestionRejected";
const SIDE_QUESTION_BLOCKED_BY_ACTIVE_TURN: &str = "sideQuestionBlockedByActiveTurn";

fn rejected(message: impl std::fmt::Display) -> AgentChatIntentError {
    AgentChatIntentError::coded(SIDE_QUESTION_REJECTED, &message.to_string())
}

impl RuntimeFacade {
    pub(super) fn exchange_side_question(
        &self,
        frame: AgentChatSideQuestionFrame,
    ) -> Result<AgentChatSideQuestionFrame, AgentChatIntentError> {
        let host_epoch = self.host_epoch().map_err(rejected)?;
        let is_ask = matches!(frame, AgentChatSideQuestionFrame::AskSideQuestion { .. });
        if let AgentChatSideQuestionFrame::AskSideQuestion {
            conversation_id, ..
        } = &frame
        {
            self.require_side_question_answerable(conversation_id)?;
        }
        let operation = if is_ask {
            self.owner_control_port()
                .map(|owner| owner.acquire_operation())
                .transpose()
                .map_err(rejected)?
        } else {
            None
        };
        let response = crate::agent_chat_side_question_api::exchange(
            &self.agent_chat_side_questions,
            host_epoch,
            frame,
        )
        .map_err(rejected)?;
        if is_ask {
            if let AgentChatSideQuestionFrame::Asked { ref record, .. } = response {
                if record.status == gent_types::AgentChatSideQuestionStatus::Pending {
                    crate::agent_chat_side_question_worker::dispatch(
                        self.agent_chat_side_questions.clone(),
                        self.agent_chat_side_question_runners.clone(),
                        self.local_model_events.clone(),
                        host_epoch,
                        record.clone(),
                        operation,
                    );
                }
            }
        }
        Ok(response)
    }

    fn require_side_question_answerable(
        &self,
        conversation_id: &str,
    ) -> Result<(), AgentChatIntentError> {
        let Some(reads) = &self.agent_chat_reads else {
            return Ok(());
        };
        let Ok(detail) = reads.detail(conversation_id) else {
            return Ok(());
        };
        if gent_core::side_question_waits_for_turn(detail.summary.selection.provider)
            && self
                .running_turn_run(conversation_id)
                .map_err(rejected)?
                .is_some()
        {
            return Err(AgentChatIntentError::coded(
                SIDE_QUESTION_BLOCKED_BY_ACTIVE_TURN,
                "this provider answers side questions only between turns",
            ));
        }
        Ok(())
    }
}
