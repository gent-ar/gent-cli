use gent_ports::LedgerError;
use gent_types::{
    AgentChatPromptOrigin, AgentChatPromptSaved, AgentChatRunId, Command, ConversationActivityFact,
    ConversationActivityScope, PlanArtifact, Receipt, StartImplementationRequest,
    StartImplementationResult,
};
use rusqlite::{Transaction, params};
use serde_json::json;

use super::super::agent_chat_ledger::save_prompt_in;
use super::super::agent_chat_queue_activity::append;
use super::super::queries::storage_error;
use super::super::reviewed_plan_values::{effort, mode, provider};

pub(super) fn implementation_prompt(
    tx: &Transaction<'_>,
    request: &StartImplementationRequest,
    plan: &PlanArtifact,
    run_id: &AgentChatRunId,
) -> Result<AgentChatPromptSaved, LedgerError> {
    let prompt = request.implementation_prompt(&plan.content);
    let origin = AgentChatPromptOrigin::User {
        client_request_id: Some(prompt.request_id.0.clone()),
    };
    save_prompt_in(tx, &prompt, Some(run_id), &origin, None)
}

pub(super) fn release(
    tx: &Transaction<'_>,
    prompt: &AgentChatPromptSaved,
) -> Result<(), LedgerError> {
    append(
        tx,
        format!("agent-chat-queue:{}:released", prompt.message.message_id),
        prompt.receipt.receipt_id.clone(),
        ConversationActivityFact::PromptReleased {
            scope: ConversationActivityScope {
                conversation_id: prompt.message.conversation_id.clone(),
                run_id: prompt.message.run_id.clone(),
                turn_id: prompt.message.turn_id.clone(),
                host_epoch: prompt.receipt.host_epoch,
                cursor: 0,
            },
            message_id: prompt.message.message_id.clone(),
        },
    )
}

pub(super) fn history_ordinal(
    tx: &Transaction<'_>,
    conversation: &str,
) -> Result<u64, LedgerError> {
    tx.query_row("SELECT COALESCE(MAX(ordinal), 0) FROM conversation_message_ordinals WHERE conversation_id = ?1", [conversation], |row| row.get(0)).map_err(storage_error)
}
pub(super) fn insert_child(
    tx: &Transaction<'_>,
    request: &StartImplementationRequest,
    child: &AgentChatRunId,
) -> Result<(), LedgerError> {
    tx.execute("INSERT INTO runs (run_id, conversation_id, parent_run_id, provider) VALUES (?1, ?2, ?3, ?4)", params![child.0, request.conversation_id.0, request.parent_run_id.0, provider(request.selection.provider)]).map_err(storage_error)?;
    tx.execute("INSERT INTO agent_chat_run_selections (run_id, provider, model, effort, mode) VALUES (?1, ?2, ?3, ?4, ?5)", params![child.0, provider(request.selection.provider), request.selection.model, effort(request.selection.effort), mode(request.selection.mode)]).map_err(storage_error).map(|_| ())
}
pub(super) fn command(request: &StartImplementationRequest) -> Command {
    Command {
        receipt_id: request.receipt_id.clone(),
        idempotency_key: request.idempotency_key.clone(),
        host_epoch: request.host_epoch,
        kind: "reviewedPlanApprove".into(),
        payload: json!({ "request": request }),
    }
}
pub(super) fn result(
    receipt: Receipt,
    request: &StartImplementationRequest,
    child: AgentChatRunId,
    ordinal: u64,
) -> StartImplementationResult {
    StartImplementationResult {
        receipt,
        conversation_id: request.conversation_id.clone(),
        plan_id: request.plan_id.clone(),
        plan_revision: request.plan_revision,
        parent_run_id: request.parent_run_id.clone(),
        implementation_run_id: child,
        selection: request.selection.clone(),
        context_policy: request.context_policy,
        context_through_ordinal: ordinal,
    }
}
