use super::{SqliteLedger, queries::storage_error};
use gent_ports::{LedgerError, PendingPermissionLedger};
use gent_types::{
    AgentChatConversationId, AgentChatRunId, PermissionDecisionBinding, PermissionDecisionRequest,
};
use rusqlite::{OptionalExtension, params};

impl PendingPermissionLedger for SqliteLedger {
    fn save_pending_permission(
        &self,
        request: &PermissionDecisionRequest,
    ) -> Result<(), LedgerError> {
        let binding = &request.binding;
        let connection = self.lock()?;
        let binding_json =
            serde_json::to_string(binding).map_err(|e| LedgerError::Storage(e.to_string()))?;
        let request_json =
            serde_json::to_string(request).map_err(|e| LedgerError::Storage(e.to_string()))?;
        connection.execute("INSERT INTO pending_provider_permissions (decision_id, conversation_id, run_id, binding_json, request_json) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(decision_id) DO NOTHING", params![binding.decision_id.0,binding.conversation_id.0,binding.run_id.0,binding_json,request_json]).map_err(storage_error)?;
        Ok(())
    }
    fn pending_permission(
        &self,
        conversation_id: &AgentChatConversationId,
        run_id: &AgentChatRunId,
    ) -> Result<Option<PermissionDecisionRequest>, LedgerError> {
        let connection = self.lock()?;
        connection.query_row("SELECT request_json FROM pending_provider_permissions WHERE conversation_id=?1 AND run_id=?2", params![conversation_id.0,run_id.0], |row| row.get::<_,String>(0)).optional().map_err(storage_error)?.map(|json| serde_json::from_str(&json).map_err(|e| LedgerError::Storage(e.to_string()))).transpose()
    }
    fn settle_pending_permission(
        &self,
        binding: &PermissionDecisionBinding,
    ) -> Result<(), LedgerError> {
        let connection = self.lock()?;
        let changed=connection.execute("DELETE FROM pending_provider_permissions WHERE decision_id=?1 AND conversation_id=?2 AND run_id=?3 AND binding_json=?4", params![binding.decision_id.0,binding.conversation_id.0,binding.run_id.0,serde_json::to_string(binding).map_err(|e| LedgerError::Storage(e.to_string()))?]).map_err(storage_error)?;
        if changed != 1 {
            return Err(LedgerError::Invariant(
                "pending permission binding is stale".into(),
            ));
        }
        Ok(())
    }
}
