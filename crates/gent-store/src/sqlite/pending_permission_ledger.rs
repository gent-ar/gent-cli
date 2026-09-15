use super::{SqliteLedger, conversation_activity_ledger, queries};
use gent_ports::{LedgerError, PendingPermissionLedger};
use gent_types::{
    AgentChatConversationId, AgentChatRunId, ConversationActivityFact, ConversationActivityScope,
    Event, PermissionDecisionBinding, PermissionDecisionRequest, ReceiptId,
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

impl PendingPermissionLedger for SqliteLedger {
    fn save_pending_permission(
        &self,
        request: &PermissionDecisionRequest,
    ) -> Result<(), LedgerError> {
        let binding = &request.binding;
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(queries::storage_error)?;
        let binding_json =
            serde_json::to_string(binding).map_err(|e| LedgerError::Storage(e.to_string()))?;
        let request_json =
            serde_json::to_string(request).map_err(|e| LedgerError::Storage(e.to_string()))?;
        let changed = transaction.execute("INSERT INTO pending_provider_permissions (decision_id, conversation_id, run_id, binding_json, request_json) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(conversation_id, run_id) DO NOTHING", params![binding.decision_id.0,binding.conversation_id.0,binding.run_id.0,binding_json,request_json]).map_err(queries::storage_error)?;
        if changed == 0 {
            let stored = transaction
                .query_row(
                    "SELECT binding_json, request_json FROM pending_provider_permissions WHERE conversation_id=?1 AND run_id=?2",
                    params![binding.conversation_id.0, binding.run_id.0],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .map_err(queries::storage_error)?;
            if stored != (binding_json, request_json) {
                return Err(LedgerError::Invariant(
                    "pending permission decision identity conflicts".into(),
                ));
            }
            return Ok(());
        }
        append_activity(&transaction, binding, true)?;
        transaction.commit().map_err(queries::storage_error)
    }
    fn pending_permission(
        &self,
        conversation_id: &AgentChatConversationId,
        run_id: &AgentChatRunId,
    ) -> Result<Option<PermissionDecisionRequest>, LedgerError> {
        let connection = self.lock()?;
        connection.query_row("SELECT request_json FROM pending_provider_permissions WHERE conversation_id=?1 AND run_id=?2", params![conversation_id.0,run_id.0], |row| row.get::<_,String>(0)).optional().map_err(queries::storage_error)?.map(|json| serde_json::from_str(&json).map_err(|e| LedgerError::Storage(e.to_string()))).transpose()
    }
    fn settle_pending_permission(
        &self,
        binding: &PermissionDecisionBinding,
    ) -> Result<(), LedgerError> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(queries::storage_error)?;
        let changed=transaction.execute("DELETE FROM pending_provider_permissions WHERE decision_id=?1 AND conversation_id=?2 AND run_id=?3 AND binding_json=?4", params![binding.decision_id.0,binding.conversation_id.0,binding.run_id.0,serde_json::to_string(binding).map_err(|e| LedgerError::Storage(e.to_string()))?]).map_err(queries::storage_error)?;
        if changed != 1 {
            return Err(LedgerError::Invariant(
                "pending permission binding is stale".into(),
            ));
        }
        append_activity(&transaction, binding, false)?;
        transaction.commit().map_err(queries::storage_error)
    }
}

fn append_activity(
    transaction: &rusqlite::Transaction<'_>,
    binding: &PermissionDecisionBinding,
    pending: bool,
) -> Result<(), LedgerError> {
    let state = if pending { "pending" } else { "settled" };
    let event = queries::append_event(
        transaction,
        &Event {
            cursor: 0,
            event_id: format!(
                "permission:{}:{}:{}:{state}",
                binding.run_id.0, binding.turn_id, binding.decision_id.0
            ),
            receipt_id: ReceiptId(format!("providerPermission:{state}")),
            host_epoch: binding.host_epoch,
            kind: format!("providerPermission{state}"),
            payload: serde_json::json!({
                "conversationId": binding.conversation_id.0,
                "runId": binding.run_id.0,
                "turnId": binding.turn_id,
                "decisionId": binding.decision_id.0,
            }),
        },
    )?;
    let scope = ConversationActivityScope {
        conversation_id: binding.conversation_id.0.clone(),
        run_id: binding.run_id.0.clone(),
        turn_id: binding.turn_id.clone(),
        host_epoch: binding.host_epoch,
        cursor: event.cursor,
    };
    let fact = if pending {
        ConversationActivityFact::DecisionPending {
            scope,
            decision_id: binding.decision_id.0.clone(),
        }
    } else {
        ConversationActivityFact::DecisionSettled {
            scope,
            decision_id: binding.decision_id.0.clone(),
        }
    };
    conversation_activity_ledger::append(transaction, &fact)
}

pub(crate) fn settle_for_turn(
    transaction: &rusqlite::Transaction<'_>,
    turn_id: &str,
) -> Result<(), LedgerError> {
    let binding_json = transaction
        .query_row(
            "SELECT binding_json FROM pending_provider_permissions WHERE json_extract(binding_json, '$.turnId') = ?1",
            params![turn_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(queries::storage_error)?;
    let Some(binding_json) = binding_json else {
        return Ok(());
    };
    let binding: PermissionDecisionBinding = serde_json::from_str(&binding_json)
        .map_err(|error| LedgerError::Storage(error.to_string()))?;
    let changed = transaction
        .execute(
            "DELETE FROM pending_provider_permissions WHERE decision_id = ?1 AND conversation_id = ?2 AND run_id = ?3 AND binding_json = ?4",
            params![
                binding.decision_id.0,
                binding.conversation_id.0,
                binding.run_id.0,
                binding_json
            ],
        )
        .map_err(queries::storage_error)?;
    if changed != 1 {
        return Err(LedgerError::Invariant(
            "pending permission terminal settlement is stale".into(),
        ));
    }
    append_activity(transaction, &binding, false)
}
