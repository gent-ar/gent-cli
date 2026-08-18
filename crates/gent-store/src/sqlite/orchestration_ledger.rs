//! Fresh-schema `SQLite` storage for typed, uncomposed orchestration graphs.

use gent_ports::{IngressMode, LedgerError, OrchestrationLedger, OrchestrationWrite};
use gent_types::{
    CrossReviewRequest, FanoutRequest, TaskGraph, TaskNode, TaskNodeStatus, TaskRole,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use super::{
    SqliteLedger,
    epoch::require_epoch,
    queries::{host_ingress, storage_error},
};

impl OrchestrationLedger for SqliteLedger {
    fn task_graph(&self, graph_id: &str) -> Result<Option<TaskGraph>, LedgerError> {
        graph(&*self.lock()?, graph_id)
    }
    fn apply_fanout(&self, request: &FanoutRequest) -> Result<OrchestrationWrite, LedgerError> {
        fanout(self, request)
    }
    fn apply_cross_review(
        &self,
        request: &CrossReviewRequest,
    ) -> Result<OrchestrationWrite, LedgerError> {
        review(self, request)
    }
}

fn fanout(
    ledger: &SqliteLedger,
    request: &FanoutRequest,
) -> Result<OrchestrationWrite, LedgerError> {
    request.validate().map_err(|_| invalid("fanout request"))?;
    let encoded = encode_fanout(request)?;
    let mut connection = ledger.lock()?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    ingress(&tx, request.graph.host_epoch)?;
    if let Some(current) = retry(&tx, &request.graph.idempotency_key, &encoded)? {
        return Ok(OrchestrationWrite::Current(current));
    }
    fences(&tx, &request.graph)?;
    if graph(&tx, &request.graph.binding.graph_id)?.is_some() {
        return Err(invalid("graph identity"));
    }
    save(&tx, &request.graph)?;
    idempotency(
        &tx,
        &request.graph.idempotency_key,
        &request.graph.binding.graph_id,
        &encoded,
    )?;
    tx.commit().map_err(storage_error)?;
    Ok(OrchestrationWrite::Created(request.graph.clone()))
}

fn review(
    ledger: &SqliteLedger,
    request: &CrossReviewRequest,
) -> Result<OrchestrationWrite, LedgerError> {
    request
        .validate()
        .map_err(|_| invalid("cross-review request"))?;
    let encoded = encode_review(request)?;
    let mut connection = ledger.lock()?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    ingress(&tx, request.host_epoch)?;
    if let Some(current) = retry(&tx, &request.idempotency_key, &encoded)? {
        return Ok(OrchestrationWrite::Current(current));
    }
    let current = graph(&tx, &request.graph_id)?.ok_or_else(|| invalid("graph"))?;
    fences(&tx, &current)?;
    if current.revision != request.expected_graph_revision
        || current.binding.root_run_id != request.expected_parent_run_id
        || current.host_epoch != request.host_epoch
        || current.binding.goal_revision != request.goal_revision
        || current.binding.policy_revision != request.policy_revision
    {
        return Err(invalid("cross-review fence"));
    }
    let candidate = current
        .nodes
        .iter()
        .find(|node| node.spec.node_id == request.candidate.node_id)
        .ok_or_else(|| invalid("candidate"))?;
    if candidate.status != TaskNodeStatus::Completed
        || candidate.revision != request.candidate.node_revision
        || candidate.result_artifact_digest.as_deref()
            != Some(&request.candidate.artifact_digest_sha256)
        || current.binding.base_revision_digest_sha256
            != request.candidate.base_revision_digest_sha256
    {
        return Err(invalid("candidate revision"));
    }
    if candidate.spec.profile.provider == request.reviewer.profile.provider
        || !matches!(request.reviewer.role, TaskRole::Reviewer)
        || current
            .nodes
            .iter()
            .any(|node| node.spec.node_id == request.reviewer.node_id)
    {
        return Err(invalid("reviewer"));
    }
    let mut next = current.clone();
    next.revision = next
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("graph revision"))?;
    let mut reviewer = request.reviewer.clone();
    reviewer.depends_on = vec![candidate.spec.node_id.clone()];
    next.nodes.push(TaskNode {
        spec: reviewer,
        revision: 1,
        status: TaskNodeStatus::Pending,
        result_artifact_digest: None,
    });
    next.validate().map_err(|_| invalid("review graph"))?;
    save(&tx, &next)?;
    idempotency(
        &tx,
        &request.idempotency_key,
        &next.binding.graph_id,
        &encoded,
    )?;
    tx.commit().map_err(storage_error)?;
    Ok(OrchestrationWrite::Updated(next))
}

fn ingress(tx: &Transaction<'_>, epoch: gent_types::HostEpoch) -> Result<(), LedgerError> {
    let state = host_ingress(tx)?;
    require_epoch(epoch, state.epoch)?;
    (state.mode != IngressMode::Closed)
        .then_some(())
        .ok_or(LedgerError::IngressClosed { epoch: state.epoch })
}
fn fences(tx: &Transaction<'_>, graph: &TaskGraph) -> Result<(), LedgerError> {
    let run = tx
        .query_row(
            "SELECT conversation_id FROM runs WHERE run_id = ?1",
            [&graph.binding.root_run_id.0],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(storage_error)?
        .flatten();
    if run.as_deref() != Some(&graph.binding.conversation_id.0) {
        return Err(invalid("root run"));
    }
    let goal = tx.query_row("SELECT 1 FROM conversation_goals WHERE goal_id = ?1 AND conversation_id = ?2 AND run_id = ?3 AND revision = ?4 AND status = 'active'", params![graph.binding.goal_id, graph.binding.conversation_id.0, graph.binding.root_run_id.0, graph.binding.goal_revision], |_| Ok(())).optional().map_err(storage_error)?;
    if goal.is_none() {
        return Err(invalid("active goal revision"));
    }
    let policy = tx.query_row("SELECT policy_id, revision FROM policies WHERE workspace_id = ?1 AND scope = 'providerPermissions' ORDER BY revision DESC LIMIT 1", [&graph.binding.workspace_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))).optional().map_err(storage_error)?;
    matches!(policy, Some((id, revision)) if id == graph.binding.policy_id && revision == graph.binding.policy_revision).then_some(()).ok_or_else(|| invalid("policy revision"))
}
fn graph(
    connection: &rusqlite::Connection,
    graph_id: &str,
) -> Result<Option<TaskGraph>, LedgerError> {
    connection
        .query_row(
            "SELECT graph_json FROM orchestration_graphs WHERE graph_id = ?1",
            [graph_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(storage_error)?
        .map(|encoded| serde_json::from_str(&encoded).map_err(|_| invalid("stored graph")))
        .transpose()
}
fn retry(tx: &Transaction<'_>, key: &str, encoded: &str) -> Result<Option<TaskGraph>, LedgerError> {
    let row = tx.query_row("SELECT graph_id, command_json FROM orchestration_idempotency WHERE idempotency_key = ?1", [key], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).optional().map_err(storage_error)?;
    row.map_or(Ok(None), |(id, previous)| {
        if previous == encoded {
            graph(tx, &id)
        } else {
            Err(invalid("idempotency"))
        }
    })
}
fn save(tx: &Transaction<'_>, value: &TaskGraph) -> Result<(), LedgerError> {
    let encoded = encode_graph(value)?;
    tx.execute("INSERT INTO orchestration_graphs (graph_id, conversation_id, root_run_id, revision, host_epoch, graph_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(graph_id) DO UPDATE SET revision = excluded.revision, host_epoch = excluded.host_epoch, graph_json = excluded.graph_json", params![value.binding.graph_id, value.binding.conversation_id.0, value.binding.root_run_id.0, value.revision, value.host_epoch.0, encoded]).map_err(storage_error).map(|_| ())
}
fn idempotency(
    tx: &Transaction<'_>,
    key: &str,
    graph_id: &str,
    command: &str,
) -> Result<(), LedgerError> {
    tx.execute("INSERT INTO orchestration_idempotency (idempotency_key, graph_id, command_json) VALUES (?1, ?2, ?3)", params![key, graph_id, command]).map_err(storage_error).map(|_| ())
}
fn encode_fanout(value: &FanoutRequest) -> Result<String, LedgerError> {
    serde_json::to_string(value).map_err(|error| LedgerError::Storage(error.to_string()))
}
fn encode_review(value: &CrossReviewRequest) -> Result<String, LedgerError> {
    serde_json::to_string(value).map_err(|error| LedgerError::Storage(error.to_string()))
}
fn encode_graph(value: &TaskGraph) -> Result<String, LedgerError> {
    serde_json::to_string(value).map_err(|error| LedgerError::Storage(error.to_string()))
}
fn invalid(subject: &str) -> LedgerError {
    LedgerError::Invariant(format!("orchestration {subject} is invalid"))
}

#[cfg(test)]
#[path = "orchestration_ledger_tests.rs"]
mod tests;
