use gent_ports::LedgerError;
use gent_types::{
    ConversationActivityFact, ConversationActivityScope, Event, HostEpoch,
    PermissionDecisionRequest, ReceiptId,
};
use rusqlite::{Transaction, TransactionBehavior, params};

use super::{SqliteLedger, require_open};
use crate::sqlite::{conversation_activity_ledger, queries};

pub(super) fn recover(ledger: &SqliteLedger, host_epoch: HostEpoch) -> Result<(), LedgerError> {
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(queries::storage_error)?;
    require_open(&transaction, host_epoch)?;
    transaction.execute(
        "UPDATE agent_chat_prompt_dispatches SET state = 'pending', coordinator_id = NULL, host_epoch = NULL WHERE state = 'claimed' AND host_epoch < ?1",
        [host_epoch.0],
    ).map_err(queries::storage_error)?;
    fail_open_turns(
        &transaction,
        "d.state IN ('launching', 'started') AND d.host_epoch < ?1",
        Some(host_epoch),
        host_epoch,
    )?;
    transaction.execute(
        "UPDATE agent_chat_prompt_dispatches SET state = 'unprovable' WHERE state IN ('launching', 'started') AND host_epoch < ?1",
        [host_epoch.0],
    ).map_err(queries::storage_error)?;
    fail_open_turns(&transaction, "d.state = 'unprovable'", None, host_epoch)?;
    recover_permissions(&transaction, host_epoch)?;
    transaction.commit().map_err(queries::storage_error)
}

fn fail_open_turns(
    transaction: &Transaction<'_>,
    dispatch_filter: &str,
    epoch_parameter: Option<HostEpoch>,
    host_epoch: HostEpoch,
) -> Result<(), LedgerError> {
    let turn_ids = {
        let mut statement = transaction
            .prepare(&format!(
                "SELECT m.turn_id FROM agent_chat_prompt_dispatches d JOIN conversation_messages m ON m.message_id = d.message_id JOIN turns t ON t.turn_id = m.turn_id WHERE {dispatch_filter} AND t.phase IN ('active', 'waitingPermission', 'waitingQuestion')"
            ))
            .map_err(queries::storage_error)?;
        let parameters: Vec<u64> = epoch_parameter.map(|epoch| epoch.0).into_iter().collect();
        let rows = statement
            .query_map(rusqlite::params_from_iter(parameters), |row| {
                row.get::<_, String>(0)
            })
            .map_err(queries::storage_error)?;
        rows.collect::<Result<Vec<String>, _>>()
            .map_err(queries::storage_error)?
    };
    for turn_id in turn_ids {
        crate::sqlite::turn_terminal::settle(
            transaction,
            &turn_id,
            host_epoch,
            gent_types::DurableTurnPhase::Failed,
        )?;
    }
    Ok(())
}

fn recover_permissions(
    transaction: &Transaction<'_>,
    host_epoch: HostEpoch,
) -> Result<(), LedgerError> {
    let requests = {
        let mut statement = transaction
            .prepare("SELECT request_json FROM pending_provider_permissions")
            .map_err(queries::storage_error)?;
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(queries::storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(queries::storage_error)?
    };
    for encoded in requests {
        let request = serde_json::from_str::<PermissionDecisionRequest>(&encoded)
            .map_err(|error| LedgerError::Storage(error.to_string()))?;
        let binding = request.binding;
        if binding.host_epoch.0 >= host_epoch.0 {
            continue;
        }
        let changed = transaction
            .execute(
                "DELETE FROM pending_provider_permissions WHERE decision_id = ?1 AND conversation_id = ?2 AND run_id = ?3",
                params![binding.decision_id.0, binding.conversation_id.0, binding.run_id.0],
            )
            .map_err(queries::storage_error)?;
        if changed != 1 {
            return Err(LedgerError::Invariant(
                "stale pending permission recovery conflicted".into(),
            ));
        }
        let event = queries::append_event(
            transaction,
            &Event {
                cursor: 0,
                event_id: format!(
                    "permission:{}:{}:recovered:{}",
                    binding.run_id.0, binding.decision_id.0, host_epoch.0
                ),
                receipt_id: ReceiptId("providerPermission:recovered".into()),
                host_epoch,
                kind: "providerPermissionRecovered".into(),
                payload: serde_json::json!({
                    "conversationId": binding.conversation_id.0,
                    "runId": binding.run_id.0,
                    "turnId": binding.turn_id,
                    "decisionId": binding.decision_id.0,
                }),
            },
        )?;
        conversation_activity_ledger::append(
            transaction,
            &ConversationActivityFact::DecisionSettled {
                scope: ConversationActivityScope {
                    conversation_id: binding.conversation_id.0,
                    run_id: binding.run_id.0,
                    turn_id: binding.turn_id,
                    host_epoch,
                    cursor: event.cursor,
                },
                decision_id: binding.decision_id.0,
            },
        )?;
    }
    Ok(())
}
