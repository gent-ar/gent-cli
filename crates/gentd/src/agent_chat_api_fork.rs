//! Daemon-only dispatch for copying a conversation's prior messages into a new one.

use gent_protocol::AgentChatIntentFrame;
use gent_runtime::{AgentChatForkResult, AgentChatForkService};
use gent_types::HostEpoch;

pub(super) fn fork<L>(
    service: &AgentChatForkService<L>,
    host_epoch: HostEpoch,
    request_id: gent_types::AgentChatRequestId,
    receipt_id: gent_types::ReceiptId,
    source_conversation_id: gent_types::AgentChatConversationId,
    fork_through_message_id: String,
) -> Result<Vec<AgentChatIntentFrame>, String>
where
    L: gent_ports::AgentChatForkLedger,
{
    match service
        .fork(&gent_types::AgentChatFork {
            request_id: request_id.clone(),
            receipt_id,
            host_epoch,
            source_conversation_id,
            fork_through_message_id,
        })
        .map_err(|error| error.to_string())?
    {
        AgentChatForkResult::Forked(forked) => Ok(vec![AgentChatIntentFrame::Forked {
            request_id,
            receipt: forked.receipt,
            source_conversation_id: forked.source_conversation_id,
            conversation_id: forked.conversation_id,
            run_id: forked.run_id,
        }]),
        AgentChatForkResult::DeniedObserver => Err("agent-chat authority is disabled".into()),
    }
}
