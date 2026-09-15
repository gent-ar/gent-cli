use gent_protocol::AgentChatIntentFrame;
use gent_runtime::{
    AgentChatSelectionGate, AgentChatSelectionSwitchRequest, AgentChatSelectionSwitchResult,
    AgentChatSelectionSwitchService,
};
use gent_types::HostEpoch;

use crate::agent_chat_intent_error::AgentChatIntentError;

pub(super) struct SwitchInput {
    pub(super) request_id: gent_types::AgentChatRequestId,
    pub(super) receipt_id: gent_types::ReceiptId,
    pub(super) conversation_id: gent_types::AgentChatConversationId,
    pub(super) parent_run_id: gent_types::AgentChatRunId,
    pub(super) selection: gent_types::AgentChatSelection,
    pub(super) context_policy: gent_types::ContextPolicy,
}

pub(super) fn switch<L, G>(
    service: &AgentChatSelectionSwitchService<L, G>,
    host_epoch: HostEpoch,
    input: SwitchInput,
) -> Result<Vec<AgentChatIntentFrame>, AgentChatIntentError>
where
    L: gent_ports::AgentChatSelectionLedger,
    G: AgentChatSelectionGate,
{
    match service.switch(&AgentChatSelectionSwitchRequest {
        request_id: input.request_id.clone(),
        receipt_id: input.receipt_id,
        host_epoch,
        conversation_id: input.conversation_id.clone(),
        parent_run_id: input.parent_run_id,
        selection: input.selection,
        context_policy: input.context_policy,
    })? {
        AgentChatSelectionSwitchResult::Switched(switched) => {
            Ok(vec![AgentChatIntentFrame::Switched {
                request_id: input.request_id,
                receipt: switched.receipt,
                conversation_id: switched.conversation_id,
                parent_run_id: switched.parent_run_id,
                run_id: switched.run_id,
                context_policy: switched.context_policy,
                context_through_ordinal: switched.context_through_ordinal,
            }])
        }
        AgentChatSelectionSwitchResult::DeniedObserver => {
            Err("agent-chat authority is disabled".into())
        }
    }
}
