use gent_ports::LedgerError;
use gent_types::{ConversationActivityFact, Event, HostEpoch, ReceiptId, ToolActivity, ToolPhase};
use rusqlite::{Connection, Transaction, params_from_iter, types::Value};

use super::{conversation_activity_ledger, queries};

const OPEN: &str = "SELECT f.cursor, f.payload FROM conversation_activity_facts f WHERE json_extract(f.payload, '$.type') = 'toolActivity' AND json_extract(f.payload, '$.activity.phase') = 'background' AND NOT EXISTS (SELECT 1 FROM conversation_activity_facts g WHERE g.conversation_id = f.conversation_id AND g.cursor > f.cursor AND json_extract(g.payload, '$.type') = 'toolActivity' AND json_extract(g.payload, '$.activity.toolUseId') = json_extract(f.payload, '$.activity.toolUseId'))";

pub(super) enum Scope<'a> {
    Run(&'a str),
    Conversation(&'a str),
    BeforeEpoch(HostEpoch),
}

pub(crate) const RESTARTED_NOTICE: &str =
    "Gentd restarted, which stopped a background command this turn had started.";

fn open(
    connection: &Connection,
    scope: &Scope<'_>,
) -> Result<Vec<(u64, ConversationActivityFact)>, LedgerError> {
    let (filter, parameter) = match scope {
        Scope::Run(run_id) => (" AND f.run_id = ?1", Value::from((*run_id).to_owned())),
        Scope::Conversation(conversation_id) => (
            " AND f.conversation_id = ?1",
            Value::from((*conversation_id).to_owned()),
        ),
        Scope::BeforeEpoch(epoch) => (
            " AND f.host_epoch < ?1",
            Value::from(i64::try_from(epoch.0).map_err(queries::storage_error)?),
        ),
    };
    let mut statement = connection
        .prepare_cached(&format!("{OPEN}{filter} ORDER BY f.cursor"))
        .map_err(queries::storage_error)?;
    statement
        .query_map(params_from_iter([parameter]), |row| {
            Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(queries::storage_error)?
        .map(|row| {
            let (cursor, payload) = row.map_err(queries::storage_error)?;
            Ok((
                cursor,
                serde_json::from_str(&payload).map_err(queries::storage_error)?,
            ))
        })
        .collect()
}

pub(super) fn any_open(connection: &Connection, run_id: &str) -> Result<bool, LedgerError> {
    Ok(!open(connection, &Scope::Run(run_id))?.is_empty())
}

pub(super) fn end(
    transaction: &Transaction<'_>,
    scope: &Scope<'_>,
    phase: &ToolPhase,
    host_epoch: Option<HostEpoch>,
) -> Result<Vec<String>, LedgerError> {
    let mut turns = Vec::new();
    for (background_cursor, fact) in open(transaction, scope)? {
        let ConversationActivityFact::ToolActivity {
            mut scope,
            activity,
        } = fact
        else {
            continue;
        };
        scope.host_epoch = host_epoch.unwrap_or(scope.host_epoch);
        scope.cursor = 0;
        let ended = ConversationActivityFact::ToolActivity {
            activity: ToolActivity {
                phase: phase.clone(),
                input: None,
                output_digest: None,
                ..activity
            },
            scope: scope.clone(),
        };
        let event = queries::append_event(
            transaction,
            &Event {
                cursor: 0,
                event_id: format!("agent-chat-background-ended:{background_cursor}"),
                receipt_id: ReceiptId(format!("agentChatBackgroundEnded:{}", scope.run_id)),
                host_epoch: scope.host_epoch,
                kind: "providerActivity".into(),
                payload: serde_json::json!({
                    "conversationId": scope.conversation_id,
                    "runId": scope.run_id,
                    "turnId": scope.turn_id,
                    "activity": ended,
                }),
            },
        )?;
        conversation_activity_ledger::append(
            transaction,
            &gent_core::with_activity_cursor(ended, event.cursor),
        )?;
        if !turns.contains(&scope.turn_id) {
            turns.push(scope.turn_id);
        }
    }
    Ok(turns)
}

pub(super) fn end_after_restart(
    transaction: &Transaction<'_>,
    host_epoch: HostEpoch,
) -> Result<(), LedgerError> {
    let ended = end(
        transaction,
        &Scope::BeforeEpoch(host_epoch),
        &ToolPhase::Failed,
        Some(host_epoch),
    )?;
    for turn_id in ended {
        super::turn_terminal::append_failure_notice(
            transaction,
            &turn_id,
            &format!(
                "{}background:{}:{turn_id}",
                gent_types::AgentChatFailureCode::GentdRestarted.event_prefix(),
                host_epoch.0
            ),
            RESTARTED_NOTICE,
        )?;
    }
    Ok(())
}
