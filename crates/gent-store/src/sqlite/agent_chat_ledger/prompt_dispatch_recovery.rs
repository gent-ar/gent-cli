use gent_ports::LedgerError;
use gent_types::{
    ConversationActivityFact, ConversationActivityScope, Event, HostEpoch,
    PermissionDecisionRequest, ReceiptId,
};
use rusqlite::{Transaction, TransactionBehavior};

use super::{SqliteLedger, require_open};
use crate::sqlite::{conversation_activity_ledger, queries};

#[path = "prompt_dispatch_install_recovery.rs"]
mod install_recovery;

pub(super) fn recover(ledger: &SqliteLedger, host_epoch: HostEpoch) -> Result<(), LedgerError> {
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(queries::storage_error)?;
    require_open(&transaction, host_epoch)?;
    install_recovery::restore(&transaction, host_epoch)?;
    crate::sqlite::agent_chat_steer_target::release_stale(&transaction, host_epoch)?;
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
    crate::sqlite::background_tools::end_after_restart(&transaction, host_epoch)?;
    crate::sqlite::agent_chat_steer_target::expire_unprovable(&transaction, host_epoch)?;
    recover_permissions(&transaction, host_epoch)?;
    super::super::switch_queue::carry_undispatched_prompts(&transaction)?;
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
        if crate::sqlite::turn_terminal::settle(
            transaction,
            &turn_id,
            host_epoch,
            gent_types::DurableTurnPhase::Failed,
        )? {
            crate::sqlite::turn_terminal::append_failure_notice(
                transaction,
                &turn_id,
                &format!(
                    "{}{turn_id}",
                    gent_types::AgentChatFailureCode::GentdRestarted.event_prefix()
                ),
                GENTD_RESTARTED_NOTICE,
            )?;
        }
    }
    Ok(())
}

const GENTD_RESTARTED_NOTICE: &str =
    "Gentd restarted while this turn was running. Send the prompt again to continue.";

fn recover_permissions(
    transaction: &Transaction<'_>,
    host_epoch: HostEpoch,
) -> Result<(), LedgerError> {
    let rows = {
        let mut statement = transaction
            .prepare("SELECT rowid, decision_id, conversation_id, run_id, request_json FROM pending_provider_permissions ORDER BY rowid")
            .map_err(queries::storage_error)?;
        statement
            .query_map([], |row| {
                Ok(PendingRow {
                    rowid: row.get(0)?,
                    decision_id: row.get(1)?,
                    conversation_id: row.get(2)?,
                    run_id: row.get(3)?,
                    request_json: row.get(4)?,
                })
            })
            .map_err(queries::storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(queries::storage_error)?
    };
    for row in rows {
        transaction
            .execute_batch("SAVEPOINT recover_permission")
            .map_err(queries::storage_error)?;
        let outcome = recover_permission(transaction, host_epoch, &row);
        if let Err(LedgerError::Busy(message)) = outcome {
            return Err(LedgerError::Busy(message));
        }
        if let Err(error) = &outcome {
            transaction
                .execute_batch("ROLLBACK TO recover_permission")
                .map_err(queries::storage_error)?;
            eprintln!(
                "gentd: pending permission recovery skipped decision {} conversation {} run {}: {error}",
                row.decision_id, row.conversation_id, row.run_id
            );
        }
        transaction
            .execute_batch("RELEASE recover_permission")
            .map_err(queries::storage_error)?;
    }
    Ok(())
}

struct PendingRow {
    rowid: i64,
    decision_id: String,
    conversation_id: String,
    run_id: String,
    request_json: String,
}

fn recover_permission(
    transaction: &Transaction<'_>,
    host_epoch: HostEpoch,
    row: &PendingRow,
) -> Result<(), LedgerError> {
    let binding = serde_json::from_str::<PermissionDecisionRequest>(&row.request_json)
        .map(|request| request.binding)
        .ok();
    if binding
        .as_ref()
        .is_some_and(|binding| binding.host_epoch.0 >= host_epoch.0)
    {
        return Ok(());
    }
    transaction
        .execute(
            "DELETE FROM pending_provider_permissions WHERE rowid = ?1",
            [row.rowid],
        )
        .map_err(queries::storage_error)?;
    let Some(binding) = binding.filter(|binding| {
        binding.decision_id.0 == row.decision_id
            && binding.conversation_id.0 == row.conversation_id
            && binding.run_id.0 == row.run_id
    }) else {
        report_unsettled(row);
        return Ok(());
    };
    let event = queries::append_event(
        transaction,
        &Event {
            cursor: 0,
            event_id: format!(
                "permission:{}:{}:recovered:{}",
                row.run_id, row.decision_id, host_epoch.0
            ),
            receipt_id: ReceiptId("providerPermission:recovered".into()),
            host_epoch,
            kind: "providerPermissionRecovered".into(),
            payload: serde_json::json!({
                "conversationId": row.conversation_id,
                "runId": row.run_id,
                "turnId": binding.turn_id,
                "decisionId": row.decision_id,
            }),
        },
    )?;
    conversation_activity_ledger::append(
        transaction,
        &ConversationActivityFact::DecisionSettled {
            scope: ConversationActivityScope {
                conversation_id: row.conversation_id.clone(),
                run_id: row.run_id.clone(),
                turn_id: binding.turn_id,
                host_epoch,
                cursor: event.cursor,
            },
            decision_id: row.decision_id.clone(),
            outcome: Some(gent_types::DecisionOutcome::Cancelled),
        },
    )
}

fn report_unsettled(row: &PendingRow) {
    eprintln!(
        "gentd: stale pending permission removed without a settlement fact; its request does not name decision {} conversation {} run {}",
        row.decision_id, row.conversation_id, row.run_id
    );
}
