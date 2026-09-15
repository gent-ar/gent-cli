use gent_ports::{LedgerError, MAX_PLAN_PURSUIT_BATCH};
use gent_types::{
    AgentChatConversationId, AgentChatRunId, ConversationActivityFact, ConversationActivityScope,
    PlanArtifact, PlanImplementation, PlanTurn, ReceiptId,
};
use rusqlite::{Connection, OptionalExtension, params};

use super::super::agent_chat_queue_activity::append_activity;
use super::super::queries::{host_ingress, storage_error};
use super::super::reviewed_plan_values::decode;

pub(super) fn current(
    connection: &Connection,
    conversation_id: &str,
) -> Result<Option<PlanArtifact>, LedgerError> {
    connection
        .query_row(
            "SELECT a.artifact_json, a.status FROM reviewed_plan_current c JOIN reviewed_plan_artifacts a ON a.plan_id = c.plan_id AND a.revision = c.revision WHERE a.conversation_id = ?1 ORDER BY a.rowid DESC LIMIT 1",
            [conversation_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(storage_error)?
        .map(|(encoded, status)| decode(&encoded, &status))
        .transpose()
}

pub(super) fn turns_awaiting_review(connection: &Connection) -> Result<Vec<PlanTurn>, LedgerError> {
    let mut statement = connection
        .prepare(
            "SELECT t.conversation_id, t.run_id, t.turn_id, (SELECT e.text FROM agent_chat_transcript_events e WHERE e.turn_id = t.turn_id AND e.is_partial = 0 AND (e.kind = 'plan' OR (s.provider = 'claurst' AND e.kind = 'assistantMessage')) ORDER BY e.kind = 'plan' DESC, e.cursor DESC LIMIT 1) FROM turns t JOIN agent_chat_run_selections s ON s.run_id = t.run_id WHERE s.mode = 'plan' AND t.phase = 'completed' AND NOT EXISTS (SELECT 1 FROM reviewed_plan_artifacts a WHERE a.source_turn_id = t.turn_id) ORDER BY t.rowid ASC LIMIT ?1",
        )
        .map_err(storage_error)?;
    let rows = statement
        .query_map(
            [i64::try_from(MAX_PLAN_PURSUIT_BATCH).unwrap_or(i64::MAX)],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)?;
    Ok(rows
        .into_iter()
        .filter_map(|(conversation_id, run_id, turn_id, content)| {
            content
                .filter(|content| !content.trim().is_empty())
                .map(|content| PlanTurn {
                    conversation_id: AgentChatConversationId(conversation_id),
                    run_id: AgentChatRunId(run_id),
                    turn_id,
                    content,
                })
        })
        .collect())
}

pub(super) fn implementations_awaiting_prompt(
    connection: &Connection,
) -> Result<Vec<PlanImplementation>, LedgerError> {
    let mut statement = connection
        .prepare(
            "SELECT r.idempotency_key, r.implementation_run_id, a.artifact_json, a.status FROM reviewed_plan_approval_receipts r JOIN reviewed_plan_artifacts a ON a.plan_id = r.plan_id AND a.revision = r.plan_revision WHERE NOT EXISTS (SELECT 1 FROM conversation_messages m WHERE m.run_id = r.implementation_run_id) AND r.implementation_run_id = (SELECT latest.run_id FROM runs latest WHERE latest.conversation_id = a.conversation_id ORDER BY latest.rowid DESC LIMIT 1) ORDER BY r.rowid ASC LIMIT ?1",
        )
        .map_err(storage_error)?;
    let rows = statement
        .query_map(
            [i64::try_from(MAX_PLAN_PURSUIT_BATCH).unwrap_or(i64::MAX)],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)?;
    rows.into_iter()
        .map(|(idempotency_key, run_id, encoded, status)| {
            Ok(PlanImplementation {
                idempotency_key,
                implementation_run_id: AgentChatRunId(run_id),
                plan: decode(&encoded, &status)?,
            })
        })
        .collect()
}

pub(super) fn publish(connection: &Connection, plan: &PlanArtifact) -> Result<(), LedgerError> {
    let host_epoch = host_ingress(connection)?.epoch;
    append_activity(
        connection,
        format!(
            "plan:{}:{}:{:?}",
            plan.plan_id.0, plan.revision.0, plan.status
        ),
        ReceiptId(format!("plan:{}:{}", plan.plan_id.0, plan.revision.0)),
        "agentChatPlanActivity",
        ConversationActivityFact::PlanUpdated {
            scope: ConversationActivityScope {
                conversation_id: plan.conversation_id.0.clone(),
                run_id: plan.source_run_id.0.clone(),
                turn_id: plan.source_turn_id.clone(),
                host_epoch,
                cursor: 0,
            },
            plan: plan.clone(),
        },
    )
}

pub(super) fn stored(
    connection: &Connection,
    plan_id: &str,
    revision: u64,
) -> Result<PlanArtifact, LedgerError> {
    let (encoded, status) = connection
        .query_row(
            "SELECT artifact_json, status FROM reviewed_plan_artifacts WHERE plan_id = ?1 AND revision = ?2",
            params![plan_id, revision],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(storage_error)?;
    decode(&encoded, &status)
}
