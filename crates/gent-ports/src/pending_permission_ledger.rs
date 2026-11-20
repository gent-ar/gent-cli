use crate::LedgerError;
use gent_types::{
    AgentChatConversationId, AgentChatRunId, DecisionOutcome, PermissionDecisionBinding,
    PermissionDecisionRequest, PolicyRecord, Receipt,
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
    fn conversation_pending_permission(
        &self,
        conversation_id: &AgentChatConversationId,
    ) -> Result<Option<PermissionDecisionRequest>, LedgerError>;
    fn settle_pending_permission(
        &self,
        binding: &PermissionDecisionBinding,
    ) -> Result<(), LedgerError>;
    fn settle_delivered_permission(
        &self,
        binding: &PermissionDecisionBinding,
        grant: Option<&PolicyRecord>,
        outcome: DecisionOutcome,
    ) -> Result<(), LedgerError>;
    fn decision_receipt(
        &self,
        binding: &PermissionDecisionBinding,
    ) -> Result<Option<Receipt>, LedgerError>;
}
