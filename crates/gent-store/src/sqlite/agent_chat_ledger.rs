//! Atomic `SQLite` ownership for an agent-chat conversation and its root run.

use gent_ports::{AgentChatLedger, IngressMode, LedgerError};
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationCreated, AgentChatEffort, AgentChatMode,
    AgentChatProvider, Command, Receipt, ReceiptStatus,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::json;

use super::SqliteLedger;
use super::epoch::require_epoch;
use super::queries::{
    find_receipt, host_ingress, insert_receipt, receipt_matches_command, storage_error,
};

impl AgentChatLedger for SqliteLedger {
    fn create_agent_chat_conversation(
        &self,
        create: &AgentChatConversationCreate,
    ) -> Result<AgentChatConversationCreated, LedgerError> {
        create_conversation(self, create)
    }
}

fn create_conversation(
    ledger: &SqliteLedger,
    create: &AgentChatConversationCreate,
) -> Result<AgentChatConversationCreated, LedgerError> {
    validate(create)?;
    let command = command_for(create);
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let ingress = host_ingress(&transaction)?;
    require_epoch(create.host_epoch, ingress.epoch)?;
    if ingress.mode == IngressMode::Closed {
        return Err(LedgerError::IngressClosed {
            epoch: ingress.epoch,
        });
    }
    if let Some(result) = existing(&transaction, create)? {
        if !receipt_matches_command(&transaction, &command)? {
            return Err(LedgerError::Invariant(
                "agent chat idempotency key is bound to another create command".into(),
            ));
        }
        return Ok(result);
    }
    if find_receipt(&transaction, &create.idempotency_key)?.is_some() {
        return Err(LedgerError::Invariant(
            "agent chat idempotency key is owned by another command".into(),
        ));
    }
    reject_receipt_id_collision(&transaction, create)?;
    insert_rows(&transaction, create)?;
    let receipt = Receipt {
        receipt_id: create.receipt_id.clone(),
        idempotency_key: create.idempotency_key.clone(),
        status: ReceiptStatus::Settled,
        host_epoch: create.host_epoch,
    };
    insert_receipt(&transaction, &receipt, &command)?;
    transaction
        .execute(
            "INSERT INTO agent_chat_create_receipts (idempotency_key, conversation_id, run_id) VALUES (?1, ?2, ?3)",
            params![create.idempotency_key, create.conversation_id.0, create.run_id.0],
        )
        .map_err(storage_error)?;
    transaction.commit().map_err(storage_error)?;
    Ok(result(receipt, create))
}

fn validate(create: &AgentChatConversationCreate) -> Result<(), LedgerError> {
    for value in [
        &create.receipt_id.0,
        &create.idempotency_key,
        &create.conversation_id.0,
        &create.run_id.0,
        &create.selection.model,
    ] {
        if value.trim().is_empty() {
            return Err(LedgerError::Invariant(
                "agent chat creation identities and model must not be empty".into(),
            ));
        }
    }
    Ok(())
}

fn existing(
    transaction: &Transaction<'_>,
    create: &AgentChatConversationCreate,
) -> Result<Option<AgentChatConversationCreated>, LedgerError> {
    let row = transaction
        .query_row(
            "SELECT r.receipt_id, r.status, r.host_epoch, c.conversation_id, c.root_run_id, c.provider, c.model, c.effort, c.mode FROM agent_chat_create_receipts a JOIN receipts r ON r.idempotency_key = a.idempotency_key JOIN agent_chat_conversations c ON c.conversation_id = a.conversation_id WHERE a.idempotency_key = ?1",
            [&create.idempotency_key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, u64>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, String>(6)?, row.get::<_, String>(7)?, row.get::<_, String>(8)?)),
        )
        .optional()
        .map_err(storage_error)?;
    let Some((receipt_id, status, epoch, conversation_id, run_id, provider, model, effort, mode)) =
        row
    else {
        return Ok(None);
    };
    if receipt_id != create.receipt_id.0
        || conversation_id != create.conversation_id.0
        || run_id != create.run_id.0
        || !selection_matches(create, &provider, &model, &effort, &mode)
    {
        return Err(LedgerError::Invariant(
            "agent chat create retry conflicts with durable ownership".into(),
        ));
    }
    let status = decode_receipt_status(&status)?;
    if status != ReceiptStatus::Settled {
        return Err(LedgerError::Invariant(
            "agent chat create receipt must be settled atomically".into(),
        ));
    }
    Ok(Some(result(
        Receipt {
            receipt_id: create.receipt_id.clone(),
            idempotency_key: create.idempotency_key.clone(),
            status,
            host_epoch: gent_types::HostEpoch(epoch),
        },
        create,
    )))
}

fn reject_receipt_id_collision(
    transaction: &Transaction<'_>,
    create: &AgentChatConversationCreate,
) -> Result<(), LedgerError> {
    let owner = transaction
        .query_row(
            "SELECT idempotency_key FROM receipts WHERE receipt_id = ?1",
            [&create.receipt_id.0],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(storage_error)?;
    if owner.is_some() {
        return Err(LedgerError::Invariant(
            "agent chat receipt id is owned by another command".into(),
        ));
    }
    Ok(())
}

fn insert_rows(
    transaction: &Transaction<'_>,
    create: &AgentChatConversationCreate,
) -> Result<(), LedgerError> {
    transaction
        .execute(
            "INSERT INTO conversations (conversation_id) VALUES (?1)",
            [&create.conversation_id.0],
        )
        .map_err(storage_error)?;
    transaction
        .execute(
            "INSERT INTO runs (run_id, conversation_id, parent_run_id, provider) VALUES (?1, ?2, NULL, ?3)",
            params![create.run_id.0, create.conversation_id.0, provider(create.selection.provider)],
        )
        .map_err(storage_error)?;
    let values = params![
        create.conversation_id.0,
        create.run_id.0,
        provider(create.selection.provider),
        create.selection.model,
        effort(create.selection.effort),
        mode(create.selection.mode),
    ];
    transaction.execute("INSERT INTO agent_chat_conversations (conversation_id, root_run_id, provider, model, effort, mode) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", values).map_err(storage_error)?;
    transaction.execute("INSERT INTO agent_chat_run_selections (run_id, provider, model, effort, mode) VALUES (?1, ?2, ?3, ?4, ?5)", params![create.run_id.0, provider(create.selection.provider), create.selection.model, effort(create.selection.effort), mode(create.selection.mode)]).map_err(storage_error)?;
    Ok(())
}

fn command_for(create: &AgentChatConversationCreate) -> Command {
    Command {
        receipt_id: create.receipt_id.clone(),
        idempotency_key: create.idempotency_key.clone(),
        host_epoch: create.host_epoch,
        kind: "agentChatCreateConversation".into(),
        payload: json!({ "conversationId": create.conversation_id.0, "runId": create.run_id.0, "selection": create.selection }),
    }
}

fn result(receipt: Receipt, create: &AgentChatConversationCreate) -> AgentChatConversationCreated {
    AgentChatConversationCreated {
        receipt,
        conversation_id: create.conversation_id.clone(),
        run_id: create.run_id.clone(),
    }
}

fn selection_matches(
    create: &AgentChatConversationCreate,
    stored_provider: &str,
    model: &str,
    stored_effort: &str,
    stored_mode: &str,
) -> bool {
    stored_provider == provider(create.selection.provider)
        && model == create.selection.model
        && stored_effort == effort(create.selection.effort)
        && stored_mode == mode(create.selection.mode)
}

const fn provider(value: AgentChatProvider) -> &'static str {
    match value {
        AgentChatProvider::Claude => "claude",
        AgentChatProvider::Codex => "codex",
        AgentChatProvider::Claurst => "claurst",
    }
}

const fn effort(value: AgentChatEffort) -> &'static str {
    match value {
        AgentChatEffort::Low => "low",
        AgentChatEffort::Medium => "medium",
        AgentChatEffort::High => "high",
    }
}

const fn mode(value: AgentChatMode) -> &'static str {
    match value {
        AgentChatMode::Ask => "ask",
        AgentChatMode::Plan => "plan",
        AgentChatMode::Agent => "agent",
    }
}

fn decode_receipt_status(value: &str) -> Result<ReceiptStatus, LedgerError> {
    match value {
        "settled" => Ok(ReceiptStatus::Settled),
        "accepted" | "unprovable" | "rejected" => Err(LedgerError::Invariant(
            "agent chat create receipt has an invalid terminal state".into(),
        )),
        _ => Err(LedgerError::Storage("unknown receipt status".into())),
    }
}
