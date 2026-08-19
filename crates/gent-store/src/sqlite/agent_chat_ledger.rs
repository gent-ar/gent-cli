//! Atomic `SQLite` ownership for an agent-chat conversation and its root run.
mod prompt;
mod prompt_dispatch;
mod receipt;
mod switch;
use super::SqliteLedger;
use super::epoch::require_epoch;
use super::queries::{
    find_receipt, host_ingress, insert_receipt, receipt_matches_command, storage_error,
};
use gent_ports::{AgentChatLedger, IngressMode, LedgerError};
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationCreated, AgentChatEffort, AgentChatMode,
    AgentChatProvider, Command, Receipt, ReceiptStatus, WorkspaceRecord,
};
use receipt::decode_create_receipt_status;
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::json;

impl AgentChatLedger for SqliteLedger {
    fn create_agent_chat_conversation(
        &self,
        create: &AgentChatConversationCreate,
    ) -> Result<AgentChatConversationCreated, LedgerError> {
        create_conversation(self, create, None)
    }
}

pub(super) fn create_conversation(
    ledger: &SqliteLedger,
    create: &AgentChatConversationCreate,
    workspace: Option<&WorkspaceRecord>,
) -> Result<AgentChatConversationCreated, LedgerError> {
    validate(create)?;
    validate_workspace(workspace)?;
    let command = command_for(create, workspace);
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
    if let Some(result) = existing(&transaction, create, workspace)? {
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
    insert_rows(&transaction, create, workspace)?;
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
    ] {
        if value.trim().is_empty() {
            return Err(LedgerError::Invariant(
                "agent chat creation identities and model must not be empty".into(),
            ));
        }
    }
    create.selection.validate().map_err(|error| {
        LedgerError::Invariant(format!("agent chat creation selection is invalid: {error}"))
    })?;
    Ok(())
}

fn validate_workspace(workspace: Option<&WorkspaceRecord>) -> Result<(), LedgerError> {
    let Some(workspace) = workspace else {
        return Ok(());
    };
    if workspace.workspace_id.trim().is_empty()
        || workspace.canonical_path.trim().is_empty()
        || workspace.workspace_id.contains('\0')
        || workspace.canonical_path.contains('\0')
    {
        return Err(LedgerError::Invariant(
            "agent chat workspace identity is invalid".into(),
        ));
    }
    Ok(())
}

fn existing(
    transaction: &Transaction<'_>,
    create: &AgentChatConversationCreate,
    workspace: Option<&WorkspaceRecord>,
) -> Result<Option<AgentChatConversationCreated>, LedgerError> {
    let row = transaction
        .query_row(
            "SELECT r.receipt_id, r.status, r.host_epoch, c.conversation_id, c.root_run_id, c.provider, c.model, c.effort, c.mode, c.workspace_id, w.canonical_path FROM agent_chat_create_receipts a JOIN receipts r ON r.idempotency_key = a.idempotency_key JOIN agent_chat_conversations c ON c.conversation_id = a.conversation_id LEFT JOIN workspaces w ON w.workspace_id = c.workspace_id WHERE a.idempotency_key = ?1",
            [&create.idempotency_key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, u64>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, String>(6)?, row.get::<_, String>(7)?, row.get::<_, String>(8)?, row.get::<_, Option<String>>(9)?, row.get::<_, Option<String>>(10)?)),
        )
        .optional()
        .map_err(storage_error)?;
    let Some((
        receipt_id,
        status,
        epoch,
        conversation_id,
        run_id,
        provider,
        model,
        effort,
        mode,
        workspace_id,
        workspace_path,
    )) = row
    else {
        return Ok(None);
    };
    if receipt_id != create.receipt_id.0
        || conversation_id != create.conversation_id.0
        || run_id != create.run_id.0
        || !selection_matches(create, &provider, &model, &effort, &mode)
        || workspace_id.as_deref() != workspace.map(|value| value.workspace_id.as_str())
        || workspace_path.as_deref() != workspace.map(|value| value.canonical_path.as_str())
    {
        return Err(LedgerError::Invariant(
            "agent chat create retry conflicts with durable ownership".into(),
        ));
    }
    let status = decode_create_receipt_status(&status)?;
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
    workspace: Option<&WorkspaceRecord>,
) -> Result<(), LedgerError> {
    if let Some(workspace) = workspace {
        transaction.execute("INSERT INTO workspaces (workspace_id, canonical_path) VALUES (?1, ?2) ON CONFLICT(workspace_id) DO NOTHING", params![workspace.workspace_id, workspace.canonical_path]).map_err(storage_error)?;
        let canonical_path = transaction
            .query_row(
                "SELECT canonical_path FROM workspaces WHERE workspace_id = ?1",
                [&workspace.workspace_id],
                |row| row.get::<_, String>(0),
            )
            .map_err(storage_error)?;
        if canonical_path != workspace.canonical_path {
            return Err(LedgerError::Invariant(
                "agent chat workspace identity conflicts with durable path".into(),
            ));
        }
    }
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
        workspace.map(|value| value.workspace_id.as_str()),
    ];
    transaction.execute("INSERT INTO agent_chat_conversations (conversation_id, root_run_id, provider, model, effort, mode, workspace_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)", values).map_err(storage_error)?;
    transaction.execute("INSERT INTO agent_chat_run_selections (run_id, provider, model, effort, mode) VALUES (?1, ?2, ?3, ?4, ?5)", params![create.run_id.0, provider(create.selection.provider), create.selection.model, effort(create.selection.effort), mode(create.selection.mode)]).map_err(storage_error)?;
    Ok(())
}

fn command_for(
    create: &AgentChatConversationCreate,
    workspace: Option<&WorkspaceRecord>,
) -> Command {
    Command {
        receipt_id: create.receipt_id.clone(),
        idempotency_key: create.idempotency_key.clone(),
        host_epoch: create.host_epoch,
        kind: "agentChatCreateConversation".into(),
        payload: json!({ "conversationId": create.conversation_id.0, "runId": create.run_id.0, "selection": create.selection, "workspaceId": workspace.map(|value| &value.workspace_id) }),
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
