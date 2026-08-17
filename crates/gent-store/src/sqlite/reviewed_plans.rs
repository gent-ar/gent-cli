//! `SQLite` authority for daemon-trusted reviewed plans and atomic child reservations.

use gent_ports::{IngressMode, LedgerError, ReviewedPlanLedger};
use gent_types::{
    AgentChatRunId, Command, ContextPolicy, PlanArtifact, PlanRevision, Receipt, ReceiptStatus,
    ReviewedPlanId, StartImplementationRequest, StartImplementationResult,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::json;

use super::SqliteLedger;
use super::epoch::require_epoch;
use super::queries::{
    find_receipt, host_ingress, insert_receipt, receipt_matches_command, storage_error,
};
use super::reviewed_plan_values::{
    child_id, context_name, decode, effort, invalid, mode, provider,
};

impl ReviewedPlanLedger for SqliteLedger {
    fn save_trusted_plan(&self, plan: &PlanArtifact) -> Result<(), LedgerError> {
        save(self, plan)
    }

    fn reviewed_plan(
        &self,
        conversation: &str,
        id: &ReviewedPlanId,
    ) -> Result<Option<PlanArtifact>, LedgerError> {
        current(&*self.lock()?, conversation, id)
    }

    fn approve_reviewed_plan(
        &self,
        request: &StartImplementationRequest,
    ) -> Result<StartImplementationResult, LedgerError> {
        approve(self, request)
    }

    fn reject_reviewed_plan(
        &self,
        id: &ReviewedPlanId,
        revision: PlanRevision,
        digest: &str,
    ) -> Result<(), LedgerError> {
        reject(self, id, revision, digest)
    }
}

fn save(ledger: &SqliteLedger, plan: &PlanArtifact) -> Result<(), LedgerError> {
    plan.validate().map_err(|_| invalid("plan artifact"))?;
    let encoded =
        serde_json::to_string(plan).map_err(|error| LedgerError::Storage(error.to_string()))?;
    let mut connection = ledger.lock()?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let source_exists = tx
        .query_row(
            "SELECT 1 FROM turns WHERE turn_id = ?1 AND conversation_id = ?2 AND run_id = ?3",
            params![
                plan.source_turn_id,
                plan.conversation_id.0,
                plan.source_run_id.0
            ],
            |_| Ok(()),
        )
        .optional()
        .map_err(storage_error)?;
    if source_exists.is_none() {
        return Err(invalid("plan source boundary"));
    }
    let existing = tx.query_row("SELECT content_digest_sha256 FROM reviewed_plan_artifacts WHERE plan_id = ?1 AND revision = ?2", params![plan.plan_id.0, plan.revision.0], |row| row.get::<_, String>(0)).optional().map_err(storage_error)?;
    if let Some(digest) = existing {
        if digest == plan.content_digest_sha256 {
            return Ok(());
        }
        return Err(invalid("plan revision conflict"));
    }
    tx.execute("INSERT INTO reviewed_plan_artifacts (plan_id, revision, conversation_id, source_run_id, source_turn_id, content_digest_sha256, artifact_json, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'readyForReview')", params![plan.plan_id.0, plan.revision.0, plan.conversation_id.0, plan.source_run_id.0, plan.source_turn_id, plan.content_digest_sha256, encoded]).map_err(storage_error)?;
    tx.execute("UPDATE reviewed_plan_artifacts SET status = 'superseded' WHERE plan_id = ?1 AND revision < ?2 AND status = 'readyForReview'", params![plan.plan_id.0, plan.revision.0]).map_err(storage_error)?;
    tx.execute("INSERT INTO reviewed_plan_current (plan_id, revision) VALUES (?1, ?2) ON CONFLICT(plan_id) DO UPDATE SET revision = excluded.revision", params![plan.plan_id.0, plan.revision.0]).map_err(storage_error)?;
    tx.commit().map_err(storage_error)
}

fn current(
    connection: &rusqlite::Connection,
    conversation: &str,
    id: &ReviewedPlanId,
) -> Result<Option<PlanArtifact>, LedgerError> {
    connection.query_row("SELECT a.artifact_json, a.status FROM reviewed_plan_current c JOIN reviewed_plan_artifacts a ON a.plan_id = c.plan_id AND a.revision = c.revision WHERE a.plan_id = ?1 AND a.conversation_id = ?2", params![id.0, conversation], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).optional().map_err(storage_error)?.map_or(Ok(None), |(encoded, status)| decode(&encoded, &status).map(Some))
}

fn approve(
    ledger: &SqliteLedger,
    request: &StartImplementationRequest,
) -> Result<StartImplementationResult, LedgerError> {
    request
        .validate()
        .map_err(|_| invalid("approval request"))?;
    let command = command(request);
    let mut connection = ledger.lock()?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let ingress = host_ingress(&tx)?;
    require_epoch(request.host_epoch, ingress.epoch)?;
    if ingress.mode == IngressMode::Closed {
        return Err(LedgerError::IngressClosed {
            epoch: ingress.epoch,
        });
    }
    if let Some(result) = existing(&tx, request, &command)? {
        return Ok(result);
    }
    if find_receipt(&tx, &request.idempotency_key)?.is_some() {
        return Err(invalid("approval idempotency ownership"));
    }
    receipt_available(&tx, request)?;
    current_parent(&tx, request)?;
    current_policy(&tx, request)?;
    exact_plan(&tx, request)?;
    let ordinal = match request.context_policy {
        ContextPolicy::Preserve => history_ordinal(&tx, &request.conversation_id.0)?,
        ContextPolicy::Clear => 0,
    };
    let run_id = AgentChatRunId(child_id(request));
    insert_child(&tx, request, &run_id)?;
    tx.execute("UPDATE reviewed_plan_artifacts SET status = 'approved' WHERE plan_id = ?1 AND revision = ?2", params![request.plan_id.0, request.plan_revision.0]).map_err(storage_error)?;
    let receipt = Receipt {
        receipt_id: request.receipt_id.clone(),
        idempotency_key: request.idempotency_key.clone(),
        status: ReceiptStatus::Settled,
        host_epoch: ingress.epoch,
    };
    insert_receipt(&tx, &receipt, &command)?;
    tx.execute("INSERT INTO reviewed_plan_approval_receipts (idempotency_key, plan_id, plan_revision, parent_run_id, implementation_run_id, context_policy, context_through_ordinal, policy_workspace_id, policy_revision) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)", params![request.idempotency_key, request.plan_id.0, request.plan_revision.0, request.parent_run_id.0, run_id.0, context_name(request.context_policy), ordinal, request.policy_workspace_id, request.policy_revision]).map_err(storage_error)?;
    tx.commit().map_err(storage_error)?;
    Ok(result(receipt, request, run_id, ordinal))
}

fn reject(
    ledger: &SqliteLedger,
    id: &ReviewedPlanId,
    revision: PlanRevision,
    digest: &str,
) -> Result<(), LedgerError> {
    let mut connection = ledger.lock()?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let changed = tx.execute("UPDATE reviewed_plan_artifacts SET status = 'rejected' WHERE plan_id = ?1 AND revision = ?2 AND content_digest_sha256 = ?3 AND status = 'readyForReview'", params![id.0, revision.0, digest]).map_err(storage_error)?;
    (changed == 1)
        .then_some(())
        .ok_or_else(|| invalid("reviewable plan revision"))?;
    tx.commit().map_err(storage_error)
}

fn existing(
    tx: &Transaction<'_>,
    request: &StartImplementationRequest,
    command: &Command,
) -> Result<Option<StartImplementationResult>, LedgerError> {
    let row = tx.query_row("SELECT r.receipt_id, r.status, r.host_epoch, a.parent_run_id, a.implementation_run_id, a.context_policy, a.context_through_ordinal, a.policy_workspace_id, a.policy_revision FROM reviewed_plan_approval_receipts a JOIN receipts r ON r.idempotency_key = a.idempotency_key WHERE a.idempotency_key = ?1", [&request.idempotency_key], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, u64>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, u64>(6)?, row.get::<_, String>(7)?, row.get::<_, u64>(8)?))).optional().map_err(storage_error)?;
    let Some((receipt_id, status, epoch, parent, child, context, ordinal, workspace, policy)) = row
    else {
        return Ok(None);
    };
    if receipt_id != request.receipt_id.0
        || status != "settled"
        || parent != request.parent_run_id.0
        || context != context_name(request.context_policy)
        || workspace != request.policy_workspace_id
        || policy != request.policy_revision
        || !receipt_matches_command(tx, command)?
    {
        return Err(invalid("approval retry conflict"));
    }
    Ok(Some(result(
        Receipt {
            receipt_id: request.receipt_id.clone(),
            idempotency_key: request.idempotency_key.clone(),
            status: ReceiptStatus::Settled,
            host_epoch: gent_types::HostEpoch(epoch),
        },
        request,
        AgentChatRunId(child),
        ordinal,
    )))
}

fn exact_plan(
    tx: &Transaction<'_>,
    request: &StartImplementationRequest,
) -> Result<(), LedgerError> {
    let plan = tx.query_row("SELECT conversation_id, source_run_id, content_digest_sha256, status FROM reviewed_plan_artifacts WHERE plan_id = ?1 AND revision = ?2", params![request.plan_id.0, request.plan_revision.0], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?))).optional().map_err(storage_error)?;
    matches!(plan, Some((conversation, parent, digest, status)) if conversation == request.conversation_id.0 && parent == request.parent_run_id.0 && digest == request.plan_content_digest_sha256 && status == "readyForReview").then_some(()).ok_or_else(|| invalid("exact reviewable plan"))
}

fn current_parent(
    tx: &Transaction<'_>,
    request: &StartImplementationRequest,
) -> Result<(), LedgerError> {
    let current = tx.query_row("SELECT q.run_id FROM agent_chat_conversations c JOIN agent_chat_run_selections q JOIN runs r ON r.run_id = q.run_id WHERE c.conversation_id = ?1 AND r.conversation_id = c.conversation_id ORDER BY r.rowid DESC LIMIT 1", [&request.conversation_id.0], |row| row.get::<_, String>(0)).optional().map_err(storage_error)?;
    (current.as_deref() == Some(&request.parent_run_id.0))
        .then_some(())
        .ok_or_else(|| invalid("current parent run"))
}

fn current_policy(
    tx: &Transaction<'_>,
    request: &StartImplementationRequest,
) -> Result<(), LedgerError> {
    let revision = tx.query_row("SELECT revision FROM policies WHERE workspace_id = ?1 AND scope = 'providerPermissions' ORDER BY revision DESC LIMIT 1", [&request.policy_workspace_id], |row| row.get::<_, u64>(0)).optional().map_err(storage_error)?;
    (revision == Some(request.policy_revision))
        .then_some(())
        .ok_or_else(|| invalid("current permission policy revision"))
}

fn receipt_available(
    tx: &Transaction<'_>,
    request: &StartImplementationRequest,
) -> Result<(), LedgerError> {
    tx.query_row(
        "SELECT 1 FROM receipts WHERE receipt_id = ?1",
        [&request.receipt_id.0],
        |_| Ok(()),
    )
    .optional()
    .map_err(storage_error)?
    .is_none()
    .then_some(())
    .ok_or_else(|| invalid("receipt identity"))
}

#[cfg(test)]
#[path = "reviewed_plans_tests.rs"]
mod tests;

fn history_ordinal(tx: &Transaction<'_>, conversation: &str) -> Result<u64, LedgerError> {
    tx.query_row("SELECT COALESCE(MAX(ordinal), 0) FROM conversation_message_ordinals WHERE conversation_id = ?1", [conversation], |row| row.get(0)).map_err(storage_error)
}
fn insert_child(
    tx: &Transaction<'_>,
    request: &StartImplementationRequest,
    child: &AgentChatRunId,
) -> Result<(), LedgerError> {
    tx.execute("INSERT INTO runs (run_id, conversation_id, parent_run_id, provider) VALUES (?1, ?2, ?3, ?4)", params![child.0, request.conversation_id.0, request.parent_run_id.0, provider(request.selection.provider)]).map_err(storage_error)?;
    tx.execute("INSERT INTO agent_chat_run_selections (run_id, provider, model, effort, mode) VALUES (?1, ?2, ?3, ?4, ?5)", params![child.0, provider(request.selection.provider), request.selection.model, effort(request.selection.effort), mode(request.selection.mode)]).map_err(storage_error).map(|_| ())
}
fn command(request: &StartImplementationRequest) -> Command {
    Command {
        receipt_id: request.receipt_id.clone(),
        idempotency_key: request.idempotency_key.clone(),
        host_epoch: request.host_epoch,
        kind: "reviewedPlanApprove".into(),
        payload: json!({ "request": request }),
    }
}
fn result(
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
