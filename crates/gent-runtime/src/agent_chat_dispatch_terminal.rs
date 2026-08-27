use gent_ports::AgentChatPromptDispatchLedger;
use gent_types::{DurableTurnPhase, HostEpoch};

use crate::{AgentChatPromptDispatchAuthority, AgentChatPromptDispatchService, RuntimeError};

impl<L: AgentChatPromptDispatchLedger> AgentChatPromptDispatchService<L> {
    pub fn settle_terminal(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        phase: DurableTurnPhase,
    ) -> Result<(), RuntimeError> {
        if self.authority == AgentChatPromptDispatchAuthority::Approved {
            self.ledger.settle_agent_chat_prompt_terminal(
                message_id,
                coordinator_id,
                host_epoch,
                phase,
            )?;
        }
        Ok(())
    }
}
