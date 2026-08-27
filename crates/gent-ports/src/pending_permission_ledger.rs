use crate::LedgerError;
use gent_types::{
    AgentChatConversationId, AgentChatRunId, PermissionDecisionBinding, PermissionDecisionRequest,
};

pub trait PendingPermissionLedger: Send + Sync {
    fn save_pending_permission(
        &self,
        request: &PermissionDecisionRequest,
    ) -> Result<(), LedgerError>;
    fn pending_permission(
        &self,
        conversation_id: &AgentChatConversationId,
        run_id: &AgentChatRunId,
    ) -> Result<Option<PermissionDecisionRequest>, LedgerError>;
    fn settle_pending_permission(
        &self,
        binding: &PermissionDecisionBinding,
    ) -> Result<(), LedgerError>;
}
