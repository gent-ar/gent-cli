use gent_ports::LedgerError;
use gent_types::{ConversationActivityFact, Event, NormalizedSessionBatch, ReceiptId};
use rusqlite::{OptionalExtension, Transaction};

use super::{
    conversation_activity_ledger,
    queries::{append_event, storage_error},
};

pub(super) fn append(
    transaction: &Transaction<'_>,
    batch: &NormalizedSessionBatch,
) -> Result<Option<u64>, LedgerError> {
    let (Some(event_id), Some(activity)) = (&batch.activity_event_id, &batch.activity) else {
        return Ok(None);
    };
    let activity = &super::turn_terminal::with_steer_cause(transaction, activity)?;
    let detached_end = detached_end(transaction, activity)?;
    let activity = detached_end.as_ref().unwrap_or(activity);
    if let ConversationActivityFact::ToolActivity { scope, .. } = activity
        && detached_end.is_none()
        && let Some(terminal) =
            super::turn_terminal::unfinished_terminal_cursor(transaction, scope)?
    {
        return Ok(Some(terminal));
    }
    if let Some(recorded) = repeated_tool_phase(transaction, activity)? {
        return Ok(Some(recorded));
    }
    if let Some(ended) = ended_child(transaction, activity)? {
        return Ok(Some(ended));
    }
    if let ConversationActivityFact::Terminal { phase, .. } = activity {
        super::turn_terminal::settle_open_work(
            transaction,
            &batch.turn_id,
            batch.host_epoch,
            phase,
        )?;
    }
    let cursor = append_event(
        transaction,
        &Event {
            cursor: 0,
            event_id: event_id.clone(),
            receipt_id: ReceiptId(format!("providerActivity:{}", batch.run_id)),
            host_epoch: batch.host_epoch,
            kind: "providerActivity".into(),
            payload: serde_json::json!({
                "conversationId": batch.conversation_id,
                "runId": scope(activity).run_id,
                "turnId": scope(activity).turn_id,
                "activity": activity,
            }),
        },
    )?
    .cursor;
    conversation_activity_ledger::append(
        transaction,
        &gent_core::with_activity_cursor(activity.clone(), cursor),
    )?;
    if let ConversationActivityFact::WorkPhase {
        scope,
        work_id,
        kind: gent_types::ActivityWorkKind::Subagent,
        phase:
            gent_types::WorkPhase::Done
            | gent_types::WorkPhase::Failed
            | gent_types::WorkPhase::Interrupted,
    } = activity
    {
        super::pending_permission_ledger::settle_for_child(
            transaction,
            &scope.conversation_id,
            &scope.run_id,
            work_id,
        )?;
    }
    Ok(Some(cursor))
}

fn ended_child(
    transaction: &Transaction<'_>,
    activity: &ConversationActivityFact,
) -> Result<Option<u64>, LedgerError> {
    let ConversationActivityFact::WorkPhase {
        scope,
        work_id,
        kind: gent_types::ActivityWorkKind::Subagent,
        ..
    } = activity
    else {
        return Ok(None);
    };
    transaction
        .query_row(
            "SELECT cursor FROM conversation_activity_facts WHERE conversation_id = ?1 AND run_id = ?2 AND json_extract(payload, '$.type') = 'workPhase' AND json_extract(payload, '$.kind') = 'subagent' AND json_extract(payload, '$.workId') = ?3 AND json_extract(payload, '$.phase') IN ('done', 'failed', 'interrupted') AND cursor > (SELECT COALESCE(MAX(s.cursor), 0) FROM conversation_activity_facts s WHERE s.conversation_id = ?1 AND s.run_id = ?2 AND json_extract(s.payload, '$.type') = 'subagentStarted' AND json_extract(s.payload, '$.childId') = ?3) ORDER BY cursor DESC LIMIT 1",
            rusqlite::params![scope.conversation_id, scope.run_id, work_id],
            |row| row.get::<_, u64>(0),
        )
        .optional()
        .map_err(storage_error)
}

fn detached_end(
    transaction: &Transaction<'_>,
    activity: &ConversationActivityFact,
) -> Result<Option<ConversationActivityFact>, LedgerError> {
    let ConversationActivityFact::ToolActivity { scope, activity } = activity else {
        return Ok(None);
    };
    if activity.phase == gent_types::ToolPhase::Background {
        return Ok(None);
    }
    let owner = transaction
        .query_row(
            "SELECT run_id, json_extract(payload, '$.turnId'), json_extract(payload, '$.activity.phase') = 'background' FROM conversation_activity_facts WHERE conversation_id = ?1 AND json_extract(payload, '$.type') = 'toolActivity' AND json_extract(payload, '$.activity.toolUseId') = ?2 ORDER BY cursor DESC LIMIT 1",
            rusqlite::params![scope.conversation_id, activity.tool_use_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            },
        )
        .optional()
        .map_err(storage_error)?;
    Ok(owner
        .filter(|(_, _, background)| *background)
        .map(
            |(run_id, turn_id, _)| ConversationActivityFact::ToolActivity {
                scope: gent_types::ConversationActivityScope {
                    run_id,
                    turn_id,
                    ..scope.clone()
                },
                activity: activity.clone(),
            },
        ))
}

fn repeated_tool_phase(
    transaction: &Transaction<'_>,
    activity: &ConversationActivityFact,
) -> Result<Option<u64>, LedgerError> {
    let ConversationActivityFact::ToolActivity { scope, activity } = activity else {
        return Ok(None);
    };
    let recorded = transaction
        .query_row(
            "SELECT cursor, json_extract(payload, '$.activity') FROM conversation_activity_facts WHERE conversation_id = ?1 AND run_id = ?2 AND json_extract(payload, '$.type') = 'toolActivity' AND json_extract(payload, '$.turnId') = ?3 AND json_extract(payload, '$.activity.toolUseId') = ?4 ORDER BY cursor DESC LIMIT 1",
            rusqlite::params![scope.conversation_id, scope.run_id, scope.turn_id, activity.tool_use_id],
            |row| Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(storage_error)?;
    let Some((cursor, recorded)) = recorded else {
        return Ok(None);
    };
    let recorded: gent_types::ToolActivity =
        serde_json::from_str(&recorded).map_err(storage_error)?;
    Ok((recorded.phase == activity.phase
        && recorded.tool_name == activity.tool_name
        && recorded.parent_tool_use_id == activity.parent_tool_use_id
        && (activity.input.is_none() || recorded.input == activity.input)
        && (activity.output_digest.is_none() || recorded.output_digest == activity.output_digest))
        .then_some(cursor))
}

pub(super) fn scope(fact: &ConversationActivityFact) -> &gent_types::ConversationActivityScope {
    fact.scope()
}
