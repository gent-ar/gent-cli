//! Append-only `SQLite` fact storage and deterministic graph reduction.

use std::collections::BTreeMap;

use gent_ports::LedgerError;
use gent_types::{
    TaskGraph, TaskGraphFact, TaskGraphFactKind, TaskGraphFactPage, TaskNode, TaskNodeStatus,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::queries::storage_error;

pub(super) const MAX_GRAPH_FACT_PAGE: u16 = 128;

pub(super) fn page(
    connection: &Connection,
    graph_id: &str,
    after_cursor: u64,
    limit: u16,
) -> Result<TaskGraphFactPage, LedgerError> {
    if limit == 0 || limit > MAX_GRAPH_FACT_PAGE {
        return Err(invalid("fact page limit"));
    }
    let mut statement = connection
        .prepare(
            "SELECT cursor, revision, idempotency_key, kind, payload FROM orchestration_graph_facts \
             WHERE graph_id = ?1 AND cursor > ?2 ORDER BY cursor ASC LIMIT ?3",
        )
        .map_err(storage_error)?;
    let rows = statement
        .query_map(
            params![graph_id, after_cursor, u64::from(limit) + 1],
            |row| {
                let kind = row.get::<_, String>(3)?;
                let payload = row.get::<_, String>(4)?;
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, String>(2)?,
                    kind,
                    payload,
                ))
            },
        )
        .map_err(storage_error)?;
    let mut facts = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)?
        .into_iter()
        .map(|(cursor, revision, idempotency_key, kind, payload)| {
            decode(graph_id, cursor, revision, idempotency_key, &kind, &payload)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let next_after_cursor = if facts.len() > usize::from(limit) {
        facts.pop();
        facts.last().map(|fact| fact.cursor)
    } else {
        None
    };
    Ok(TaskGraphFactPage {
        facts,
        next_after_cursor,
    })
}

pub(super) fn graph(
    connection: &Connection,
    graph_id: &str,
) -> Result<Option<TaskGraph>, LedgerError> {
    let page = page(connection, graph_id, 0, MAX_GRAPH_FACT_PAGE)?;
    if page.facts.is_empty() {
        return Ok(None);
    }
    if page.next_after_cursor.is_some() {
        return Err(invalid("graph fact count"));
    }
    reduce(&page.facts).map(Some)
}

pub(super) fn append(
    tx: &Transaction<'_>,
    graph_id: &str,
    revision: u64,
    idempotency_key: &str,
    kind: &TaskGraphFactKind,
) -> Result<(), LedgerError> {
    let (name, payload) = encode(kind)?;
    tx.execute(
        "INSERT INTO orchestration_graph_facts (graph_id, revision, idempotency_key, kind, payload) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![graph_id, revision, idempotency_key, name, payload],
    )
    .map_err(storage_error)
    .map(|_| ())
}

pub(super) fn reduce(facts: &[TaskGraphFact]) -> Result<TaskGraph, LedgerError> {
    let first = facts
        .first()
        .ok_or_else(|| invalid("missing graph creation"))?;
    let TaskGraphFactKind::Created {
        binding,
        host_epoch,
        idempotency_key,
    } = &first.kind
    else {
        return Err(invalid("first graph fact"));
    };
    if first.revision != 1
        || first.graph_id != binding.graph_id
        || first.idempotency_key != *idempotency_key
    {
        return Err(invalid("graph creation"));
    }
    let mut nodes = BTreeMap::<String, TaskNode>::new();
    let mut graph_revision = first.revision;
    for fact in &facts[1..] {
        if fact.graph_id != binding.graph_id || fact.revision < graph_revision {
            return Err(invalid("fact ordering"));
        }
        graph_revision = fact.revision;
        match &fact.kind {
            TaskGraphFactKind::Created { .. } => return Err(invalid("duplicate creation")),
            TaskGraphFactKind::NodeAdded { node } => {
                if nodes
                    .insert(node.spec.node_id.clone(), node.clone())
                    .is_some()
                {
                    return Err(invalid("duplicate node"));
                }
            }
            TaskGraphFactKind::NodeStatusChanged {
                node_id,
                node_revision,
                status,
                result_artifact_digest,
            } => {
                let node = nodes
                    .get_mut(node_id)
                    .ok_or_else(|| invalid("status node"))?;
                if *node_revision <= node.revision {
                    return Err(invalid("status revision"));
                }
                node.revision = *node_revision;
                node.status = *status;
                node.result_artifact_digest
                    .clone_from(result_artifact_digest);
            }
            TaskGraphFactKind::ReviewAccepted {
                candidate,
                reviewer_node_id,
            } => {
                let node = nodes
                    .get(&candidate.node_id)
                    .ok_or_else(|| invalid("review candidate"))?;
                if node.revision != candidate.node_revision
                    || node.status != TaskNodeStatus::Completed
                    || node.result_artifact_digest.as_deref()
                        != Some(&candidate.artifact_digest_sha256)
                    || reviewer_node_id.is_empty()
                {
                    return Err(invalid("review fact"));
                }
            }
        }
    }
    let graph = TaskGraph {
        binding: binding.clone(),
        revision: graph_revision,
        host_epoch: *host_epoch,
        idempotency_key: idempotency_key.clone(),
        nodes: nodes.into_values().collect(),
    };
    graph.validate().map_err(|_| invalid("reduced graph"))?;
    Ok(graph)
}

fn encode(kind: &TaskGraphFactKind) -> Result<(&'static str, String), LedgerError> {
    let name = match kind {
        TaskGraphFactKind::Created { .. } => "created",
        TaskGraphFactKind::NodeAdded { .. } => "nodeAdded",
        TaskGraphFactKind::NodeStatusChanged { .. } => "nodeStatusChanged",
        TaskGraphFactKind::ReviewAccepted { .. } => "reviewAccepted",
    };
    serde_json::to_string(kind)
        .map(|payload| (name, payload))
        .map_err(|error| LedgerError::Storage(error.to_string()))
}

fn decode(
    graph_id: &str,
    cursor: u64,
    revision: u64,
    idempotency_key: String,
    kind: &str,
    payload: &str,
) -> Result<TaskGraphFact, LedgerError> {
    let value: TaskGraphFactKind =
        serde_json::from_str(payload).map_err(|_| invalid("stored fact"))?;
    let expected = match &value {
        TaskGraphFactKind::Created { .. } => "created",
        TaskGraphFactKind::NodeAdded { .. } => "nodeAdded",
        TaskGraphFactKind::NodeStatusChanged { .. } => "nodeStatusChanged",
        TaskGraphFactKind::ReviewAccepted { .. } => "reviewAccepted",
    };
    if expected != kind {
        return Err(invalid("fact kind"));
    }
    Ok(TaskGraphFact {
        graph_id: graph_id.into(),
        cursor,
        revision,
        idempotency_key,
        kind: value,
    })
}

pub(super) fn has_graph(tx: &Transaction<'_>, graph_id: &str) -> Result<bool, LedgerError> {
    tx.query_row(
        "SELECT 1 FROM orchestration_graph_facts WHERE graph_id = ?1 LIMIT 1",
        [graph_id],
        |_| Ok(()),
    )
    .optional()
    .map_err(storage_error)
    .map(|row| row.is_some())
}

pub(super) fn invalid(subject: &str) -> LedgerError {
    LedgerError::Invariant(format!("orchestration {subject} is invalid"))
}
