use gent_protocol::AgentChatSideQuestionFrame;

use super::RuntimeFacade;

impl RuntimeFacade {
    pub(super) fn exchange_side_question(
        &self,
        frame: AgentChatSideQuestionFrame,
    ) -> Result<AgentChatSideQuestionFrame, String> {
        let host_epoch = self.host_epoch()?;
        let is_ask = matches!(frame, AgentChatSideQuestionFrame::AskSideQuestion { .. });
        let operation = if is_ask {
            self.owner_control_port()
                .map(|owner| owner.acquire_operation())
                .transpose()?
        } else {
            None
        };
        let response = crate::agent_chat_side_question_api::exchange(
            &self.agent_chat_side_questions,
            host_epoch,
            frame,
        )?;
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
}
