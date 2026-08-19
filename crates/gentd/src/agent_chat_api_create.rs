//! Daemon-only workspace canonicalization for agent-chat conversation creation.

use std::path::Path;

use gent_protocol::AgentChatIntentFrame;
use gent_runtime::{
    AgentChatConversationRequest, AgentChatConversationResult, AgentChatConversationService,
};
use gent_types::HostEpoch;

use crate::workspace_identity::CanonicalWorkspace;

pub(super) fn create<L>(
    service: &AgentChatConversationService<L>,
    host_epoch: HostEpoch,
    request_id: gent_types::AgentChatRequestId,
    receipt_id: gent_types::ReceiptId,
    workspace_path: &str,
    selection: gent_types::AgentChatSelection,
) -> Result<Vec<AgentChatIntentFrame>, String>
where
    L: gent_ports::AgentChatLedger + gent_ports::AgentChatWorkspaceLedger,
{
    let workspace = CanonicalWorkspace::from_path(Path::new(workspace_path))
        .map_err(|_| "agent-chat workspace must be an accessible local directory".to_owned())?;
    match service
        .create(&AgentChatConversationRequest {
            request_id: request_id.clone(),
            receipt_id,
            host_epoch,
            selection,
            workspace: workspace.record().clone(),
        })
        .map_err(|error| error.to_string())?
    {
        AgentChatConversationResult::Created(created) => Ok(vec![AgentChatIntentFrame::Created {
            request_id,
            receipt: created.receipt,
            conversation_id: created.conversation_id,
            run_id: created.run_id,
        }]),
        AgentChatConversationResult::DeniedObserver => {
            Err("agent-chat authority is disabled".into())
        }
    }
}
