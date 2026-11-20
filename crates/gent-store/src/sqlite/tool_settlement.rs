use gent_ports::LedgerError;
use gent_types::{
    ActivityWorkKind, ConversationActivityFact, Event, HostEpoch, ReceiptId, ToolActivity,
    ToolPhase, WorkPhase,
};
use rusqlite::{Transaction, params};

use super::{conversation_activity_ledger, queries};

const OPEN_TOOLS: &str = "SELECT f.payload FROM conversation_activity_facts f JOIN turns t ON t.conversation_id = f.conversation_id AND t.run_id = f.run_id WHERE t.turn_id = ?1 AND json_extract(f.payload, '$.type') = 'toolActivity' AND json_extract(f.payload, '$.turnId') = ?1 AND json_extract(f.payload, '$.activity.phase') IN ('started', 'waitingPermission') AND f.cursor = (SELECT MAX(g.cursor) FROM conversation_activity_facts g WHERE g.conversation_id = f.conversation_id AND g.run_id = f.run_id AND json_extract(g.payload, '$.type') = 'toolActivity' AND json_extract(g.payload, '$.turnId') = ?1 AND json_extract(g.payload, '$.activity.toolUseId') = json_extract(f.payload, '$.activity.toolUseId')) ORDER BY f.cursor";

const OPEN_CHILDREN: &str = "SELECT f.payload FROM conversation_activity_facts f JOIN turns t ON t.conversation_id = f.conversation_id AND t.run_id = f.run_id WHERE t.turn_id = ?1 AND json_extract(f.payload, '$.type') = 'subagentStarted' AND json_extract(f.payload, '$.turnId') = ?1 AND f.cursor = (SELECT MIN(s.cursor) FROM conversation_activity_facts s WHERE s.conversation_id = f.conversation_id AND s.run_id = f.run_id AND json_extract(s.payload, '$.type') = 'subagentStarted' AND json_extract(s.payload, '$.childId') = json_extract(f.payload, '$.childId')) AND NOT EXISTS(SELECT 1 FROM conversation_activity_facts g WHERE g.conversation_id = f.conversation_id AND g.run_id = f.run_id AND json_extract(g.payload, '$.type') = 'workPhase' AND json_extract(g.payload, '$.kind') = 'subagent' AND json_extract(g.payload, '$.workId') = json_extract(f.payload, '$.childId') AND json_extract(g.payload, '$.phase') IN ('done', 'failed', 'interrupted')) ORDER BY f.cursor";

pub(super) fn settle_unfinished_turn(
    transaction: &Transaction<'_>,
    turn_id: &str,
    host_epoch: HostEpoch,
) -> Result<(), LedgerError> {
    super::transcript_settlement::supersede_unfinished_reply(transaction, turn_id)?;
    for fact in open_facts(transaction, OPEN_TOOLS, turn_id)? {
        let ConversationActivityFact::ToolActivity {
            mut scope,
            activity,
        } = fact
        else {
            continue;
        };
        scope.host_epoch = host_epoch;
        scope.cursor = 0;
        let settled = ConversationActivityFact::ToolActivity {
            activity: ToolActivity {
                phase: ToolPhase::Failed,
                input: None,
                output_digest: None,
                ..activity
            },
            scope,
        };
        append(transaction, turn_id, "tool", settled)?;
    }
    for fact in open_facts(transaction, OPEN_CHILDREN, turn_id)? {
        let ConversationActivityFact::SubagentStarted {
            mut scope,
            child_id,
            ..
        } = fact
        else {
            continue;
        };
        super::pending_permission_ledger::settle_for_child(
            transaction,
            &scope.conversation_id,
            &scope.run_id,
            &child_id,
        )?;
        scope.host_epoch = host_epoch;
        scope.cursor = 0;
        let ended = ConversationActivityFact::WorkPhase {
            scope,
            work_id: child_id,
            kind: ActivityWorkKind::Subagent,
            phase: WorkPhase::Interrupted,
        };
        append(transaction, turn_id, "child", ended)?;
    }
    Ok(())
}

fn open_facts(
    transaction: &Transaction<'_>,
    query: &str,
    turn_id: &str,
) -> Result<Vec<ConversationActivityFact>, LedgerError> {
    let mut statement = transaction
        .prepare_cached(query)
        .map_err(queries::storage_error)?;
    statement
        .query_map(params![turn_id], |row| row.get::<_, String>(0))
        .map_err(queries::storage_error)?
        .map(|payload| {
            serde_json::from_str(&payload.map_err(queries::storage_error)?)
                .map_err(queries::storage_error)
        })
        .collect()
}

fn append(
    transaction: &Transaction<'_>,
    turn_id: &str,
    settled: &str,
    fact: ConversationActivityFact,
) -> Result<(), LedgerError> {
    let (scope, id) = match &fact {
        ConversationActivityFact::ToolActivity { scope, activity } => {
            (scope, activity.tool_use_id.as_str())
        }
        ConversationActivityFact::WorkPhase { scope, work_id, .. } => (scope, work_id.as_str()),
        _ => return Ok(()),
    };
    let event = queries::append_event(
        transaction,
        &Event {
            cursor: 0,
            event_id: format!("agent-chat-{settled}-settled:{turn_id}:{id}"),
            receipt_id: ReceiptId(format!("agentChatTerminal:{turn_id}")),
            host_epoch: scope.host_epoch,
            kind: "providerActivity".into(),
            payload: serde_json::json!({
                "conversationId": scope.conversation_id,
                "runId": scope.run_id,
                "turnId": scope.turn_id,
                "activity": fact,
            }),
        },
    )?;
    conversation_activity_ledger::append(
        transaction,
        &gent_core::with_activity_cursor(fact, event.cursor),
    )
}
