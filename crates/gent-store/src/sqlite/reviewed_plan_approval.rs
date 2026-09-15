use gent_ports::LedgerError;
use gent_types::{
    AgentChatRunId, Command, Receipt, StartImplementationRequest, StartImplementationResult,
};
use rusqlite::{Transaction, params};
use serde_json::json;

use super::super::queries::storage_error;
use super::super::reviewed_plan_values::{effort, mode, provider};

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
