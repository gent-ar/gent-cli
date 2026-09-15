use gent_ports::LedgerError;
use gent_types::{
    AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatProvider, AgentChatRunId,
    ConversationActivityFact, ConversationActivityScope, DurableTurnPhase, HostEpoch,
};
use rusqlite::{
    OptionalExtension, ToSql, Transaction, TransactionBehavior, params, params_from_iter,
};

use super::super::super::SqliteLedger;
use super::super::super::agent_chat_queue_activity::append;
use super::super::super::queries::storage_error;
use super::super::super::turn_terminal;
use super::helpers;
use super::helpers::saved;
use super::require_open;

fn cancel_inactive_continuation(
    transaction: &Transaction<'_>,
    message_id: &str,
    host_epoch: HostEpoch,
) -> Result<bool, LedgerError> {
    let inactive = transaction
        .query_row(
            "SELECT 1 FROM agent_chat_transcript_events te JOIN conversation_goals g ON g.goal_id = json_extract(te.origin_json, '$.goalId') WHERE te.event_id = 'user:' || ?1 AND json_extract(te.origin_json, '$.kind') = 'goalContinuation' AND g.status <> 'active'",
            [message_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(storage_error)?;
    if inactive.is_none() {
        return Ok(false);
    }
    let prompt = saved(transaction, message_id)?;
    transaction
        .execute(
            "UPDATE agent_chat_prompt_dispatches SET state = 'settled', coordinator_id = NULL, host_epoch = NULL WHERE message_id = ?1 AND state = 'pending'",
            [message_id],
        )
        .map_err(storage_error)?;
    turn_terminal::settle(
        transaction,
        &prompt.message.turn_id,
        host_epoch,
        DurableTurnPhase::Cancelled,
    )?;
    append(
        transaction,
        format!("agent-chat-queue:{message_id}:canceled"),
        prompt.receipt.receipt_id.clone(),
        ConversationActivityFact::PromptCanceled {
            scope: ConversationActivityScope {
                conversation_id: prompt.message.conversation_id.clone(),
                run_id: prompt.message.run_id.clone(),
                turn_id: prompt.message.turn_id.clone(),
                host_epoch,
                cursor: 0,
            },
            message_id: message_id.into(),
        },
    )?;
    Ok(true)
}

pub(super) fn claim_excluding_runs(
    ledger: &SqliteLedger,
    coordinator_id: &str,
    host_epoch: HostEpoch,
    provider: AgentChatProvider,
    excluded_run_ids: &[AgentChatRunId],
) -> Result<Option<AgentChatPromptSaved>, LedgerError> {
    helpers::valid_owner(coordinator_id)?;
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    require_open(&transaction, host_epoch)?;
    let excluded = (!excluded_run_ids.is_empty()).then(|| {
        format!(
            " AND m.run_id NOT IN ({})",
            std::iter::repeat_n("?", excluded_run_ids.len())
                .collect::<Vec<_>>()
                .join(", ")
        )
    });
    let query = format!(
        "SELECT d.message_id FROM agent_chat_prompt_dispatches d JOIN conversation_messages m ON m.message_id = d.message_id JOIN agent_chat_run_selections s ON s.run_id = m.run_id WHERE s.provider = ?1 AND d.state = 'pending' AND m.run_id = (SELECT current.run_id FROM runs current JOIN agent_chat_run_selections selected ON selected.run_id = current.run_id WHERE current.conversation_id = m.conversation_id ORDER BY current.rowid DESC LIMIT 1){} ORDER BY d.created_rowid LIMIT 1",
        excluded.unwrap_or_default(),
    );
    let provider = helpers::provider_name(provider);
    let mut parameters: Vec<&dyn ToSql> = vec![&provider];
    parameters.extend(
        excluded_run_ids
            .iter()
            .map(|run_id| &run_id.0 as &dyn ToSql),
    );
    let message_id = loop {
        let candidate = transaction
            .query_row(&query, params_from_iter(parameters.iter()), |row| {
                row.get::<_, String>(0)
            })
            .optional()
            .map_err(storage_error)?;
        match candidate {
            Some(message_id)
                if cancel_inactive_continuation(&transaction, &message_id, host_epoch)? => {}
            candidate => break candidate,
        }
    };
    let Some(message_id) = message_id else {
        transaction.commit().map_err(storage_error)?;
        return Ok(None);
    };
    transaction.execute(
        "UPDATE agent_chat_prompt_dispatches SET state = 'claimed', coordinator_id = ?1, host_epoch = ?2 WHERE message_id = ?3 AND state = 'pending'",
        params![coordinator_id, host_epoch.0, message_id],
    ).map_err(storage_error)?;
    let saved = helpers::saved(&transaction, &message_id)?;
    if saved.disposition == AgentChatPromptDisposition::Queue {
        append(
            &transaction,
            format!("agent-chat-queue:{message_id}:released"),
            saved.receipt.receipt_id.clone(),
            ConversationActivityFact::PromptReleased {
                scope: ConversationActivityScope {
                    conversation_id: saved.message.conversation_id.clone(),
                    run_id: saved.message.run_id.clone(),
                    turn_id: saved.message.turn_id.clone(),
                    host_epoch,
                    cursor: 0,
                },
                message_id: message_id.clone(),
            },
        )?;
    }
    transaction.commit().map_err(storage_error)?;
    Ok(Some(saved))
}

#[cfg(test)]
#[path = "prompt_dispatch_claim_tests.rs"]
mod tests;
