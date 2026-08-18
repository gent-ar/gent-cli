//! Atomic `SQLite` persistence for an immutable selected child run.

use gent_ports::{AgentChatSelectionLedger, IngressMode, LedgerError};
use gent_types::{
    AgentChatSelection, AgentChatSelectionSwitch, AgentChatSelectionSwitched, Command, Receipt,
    ReceiptStatus,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::json;

use super::super::SqliteLedger;
use super::super::epoch::require_epoch;
use super::super::queries::{
    find_receipt, host_ingress, insert_receipt, receipt_matches_command, storage_error,
};

impl AgentChatSelectionLedger for SqliteLedger {
    fn switch_agent_chat_selection(
        &self,
        switch: &AgentChatSelectionSwitch,
    ) -> Result<AgentChatSelectionSwitched, LedgerError> {
        persist(self, switch)
    }
}

fn persist(
    ledger: &SqliteLedger,
    switch: &AgentChatSelectionSwitch,
) -> Result<AgentChatSelectionSwitched, LedgerError> {
    validate(switch)?;
    let command = command_for(switch);
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let ingress = host_ingress(&transaction)?;
    require_epoch(switch.host_epoch, ingress.epoch)?;
    if ingress.mode == IngressMode::Closed {
        return Err(LedgerError::IngressClosed {
            epoch: ingress.epoch,
        });
    }
    if let Some(result) = existing(&transaction, switch)? {
        if !receipt_matches_command(&transaction, &command)? {
            return Err(conflict());
        }
        return Ok(result);
    }
    if find_receipt(&transaction, &switch.idempotency_key)?.is_some() {
        return Err(LedgerError::Invariant(
            "agent chat switch idempotency key is owned by another command".into(),
        ));
    }
    reject_receipt_collision(&transaction, switch)?;
    require_current_parent(&transaction, switch)?;
    let context_through_ordinal = context_boundary(
        &transaction,
        &switch.conversation_id.0,
        switch.context_policy,
    )?;
    insert_run(&transaction, switch)?;
    let receipt = Receipt {
        receipt_id: switch.receipt_id.clone(),
        idempotency_key: switch.idempotency_key.clone(),
        status: ReceiptStatus::Settled,
        host_epoch: switch.host_epoch,
    };
    insert_receipt(&transaction, &receipt, &command)?;
    transaction.execute("INSERT INTO agent_chat_selection_switch_receipts (idempotency_key, conversation_id, parent_run_id, run_id, context_policy, context_through_ordinal) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![switch.idempotency_key, switch.conversation_id.0, switch.parent_run_id.0, switch.run_id.0, context_policy(switch.context_policy), context_through_ordinal]).map_err(storage_error)?;
    transaction.commit().map_err(storage_error)?;
    Ok(result(receipt, switch, context_through_ordinal))
}

fn validate(switch: &AgentChatSelectionSwitch) -> Result<(), LedgerError> {
    if [
        &switch.receipt_id.0,
        &switch.idempotency_key,
        &switch.conversation_id.0,
        &switch.parent_run_id.0,
        &switch.run_id.0,
    ]
    .into_iter()
    .any(|value| value.trim().is_empty())
        || switch.run_id == switch.parent_run_id
    {
        return Err(LedgerError::Invariant(
            "agent chat switch identities and model must be nonempty and distinct".into(),
        ));
    }
    switch.selection.validate().map_err(|error| {
        LedgerError::Invariant(format!("agent chat switch selection is invalid: {error}"))
    })?;
    Ok(())
}

fn existing(
    transaction: &Transaction<'_>,
    switch: &AgentChatSelectionSwitch,
) -> Result<Option<AgentChatSelectionSwitched>, LedgerError> {
    let row = transaction.query_row("SELECT r.receipt_id, r.status, r.host_epoch, s.conversation_id, s.parent_run_id, s.run_id, s.context_policy, s.context_through_ordinal, q.provider, q.model, q.effort, q.mode FROM agent_chat_selection_switch_receipts s JOIN receipts r ON r.idempotency_key = s.idempotency_key JOIN agent_chat_run_selections q ON q.run_id = s.run_id WHERE s.idempotency_key = ?1", [&switch.idempotency_key], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, u64>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, String>(6)?, row.get::<_, u64>(7)?, row.get::<_, String>(8)?, row.get::<_, String>(9)?, row.get::<_, String>(10)?, row.get::<_, String>(11)?))).optional().map_err(storage_error)?;
    let Some((
        receipt_id,
        status,
        epoch,
        conversation,
        parent,
        run,
        policy,
        boundary,
        provider,
        model,
        effort,
        mode,
    )) = row
    else {
        return Ok(None);
    };
    if receipt_id != switch.receipt_id.0
        || conversation != switch.conversation_id.0
        || parent != switch.parent_run_id.0
        || run != switch.run_id.0
        || policy != context_policy(switch.context_policy)
        || !selection_matches(&switch.selection, &provider, &model, &effort, &mode)
        || status != "settled"
    {
        return Err(conflict());
    }
    Ok(Some(result(
        Receipt {
            receipt_id: switch.receipt_id.clone(),
            idempotency_key: switch.idempotency_key.clone(),
            status: ReceiptStatus::Settled,
            host_epoch: gent_types::HostEpoch(epoch),
        },
        switch,
        boundary,
    )))
}

fn conflict() -> LedgerError {
    LedgerError::Invariant("agent chat switch retry conflicts with durable ownership".into())
}

fn selection_matches(
    selection: &AgentChatSelection,
    provider: &str,
    model: &str,
    effort: &str,
    mode: &str,
) -> bool {
    provider == super::provider(selection.provider)
        && model == selection.model
        && effort == super::effort(selection.effort)
        && mode == super::mode(selection.mode)
}

fn reject_receipt_collision(
    transaction: &Transaction<'_>,
    switch: &AgentChatSelectionSwitch,
) -> Result<(), LedgerError> {
    let owner = transaction
        .query_row(
            "SELECT 1 FROM receipts WHERE receipt_id = ?1",
            [&switch.receipt_id.0],
            |_| Ok(()),
        )
        .optional()
        .map_err(storage_error)?;
    owner.is_none().then_some(()).ok_or_else(|| {
        LedgerError::Invariant("agent chat receipt id is owned by another command".into())
    })
}

fn require_current_parent(
    transaction: &Transaction<'_>,
    switch: &AgentChatSelectionSwitch,
) -> Result<(), LedgerError> {
    let current = transaction.query_row("SELECT q.run_id FROM agent_chat_conversations c JOIN agent_chat_run_selections q JOIN runs r ON r.run_id = q.run_id WHERE c.conversation_id = ?1 AND r.conversation_id = c.conversation_id ORDER BY r.rowid DESC LIMIT 1", [&switch.conversation_id.0], |row| row.get::<_, String>(0)).optional().map_err(storage_error)?;
    (current.as_deref() == Some(&switch.parent_run_id.0))
        .then_some(())
        .ok_or_else(|| {
            LedgerError::Invariant("agent chat switch parent is not the durable current run".into())
        })
}

fn context_boundary(
    transaction: &Transaction<'_>,
    conversation_id: &str,
    policy: gent_types::ContextPolicy,
) -> Result<u64, LedgerError> {
    if policy == gent_types::ContextPolicy::Clear {
        return Ok(0);
    }
    transaction.query_row("SELECT COALESCE(MAX(ordinal), 0) FROM conversation_message_ordinals WHERE conversation_id = ?1", [conversation_id], |row| row.get::<_, u64>(0)).map_err(storage_error)
}

fn insert_run(
    transaction: &Transaction<'_>,
    switch: &AgentChatSelectionSwitch,
) -> Result<(), LedgerError> {
    transaction.execute("INSERT INTO runs (run_id, conversation_id, parent_run_id, provider) VALUES (?1, ?2, ?3, ?4)", params![switch.run_id.0, switch.conversation_id.0, switch.parent_run_id.0, super::provider(switch.selection.provider)]).map_err(storage_error)?;
    transaction.execute("INSERT INTO agent_chat_run_selections (run_id, provider, model, effort, mode) VALUES (?1, ?2, ?3, ?4, ?5)", params![switch.run_id.0, super::provider(switch.selection.provider), switch.selection.model, super::effort(switch.selection.effort), super::mode(switch.selection.mode)]).map_err(storage_error)?;
    Ok(())
}

fn command_for(switch: &AgentChatSelectionSwitch) -> Command {
    Command {
        receipt_id: switch.receipt_id.clone(),
        idempotency_key: switch.idempotency_key.clone(),
        host_epoch: switch.host_epoch,
        kind: "agentChatSwitchSelection".into(),
        payload: json!({ "conversationId": switch.conversation_id.0, "parentRunId": switch.parent_run_id.0, "runId": switch.run_id.0, "selection": switch.selection, "contextPolicy": switch.context_policy }),
    }
}

fn result(
    receipt: Receipt,
    switch: &AgentChatSelectionSwitch,
    boundary: u64,
) -> AgentChatSelectionSwitched {
    AgentChatSelectionSwitched {
        receipt,
        conversation_id: switch.conversation_id.clone(),
        parent_run_id: switch.parent_run_id.clone(),
        run_id: switch.run_id.clone(),
        selection: switch.selection.clone(),
        context_policy: switch.context_policy,
        context_through_ordinal: boundary,
    }
}

fn context_policy(policy: gent_types::ContextPolicy) -> &'static str {
    match policy {
        gent_types::ContextPolicy::Preserve => "preserve",
        gent_types::ContextPolicy::Clear => "clear",
    }
}
