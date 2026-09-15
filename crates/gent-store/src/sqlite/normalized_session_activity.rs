//! Activity portions of an atomic normalized-session write.

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
    if let Some(recorded) = repeated_tool_phase(transaction, activity)? {
        return Ok(Some(recorded));
    }
    if let ConversationActivityFact::Terminal { phase, .. } = activity {
        if *phase != gent_types::TurnPhase::Ready {
            super::transcript_settlement::supersede_unfinished_reply(transaction, &batch.turn_id)?;
        }
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
                "runId": batch.run_id,
                "turnId": batch.turn_id,
                "activity": activity,
            }),
        },
    )?
    .cursor;
    conversation_activity_ledger::append(
        transaction,
        &gent_core::with_activity_cursor(activity.clone(), cursor),
    )?;
    Ok(Some(cursor))
}

fn repeated_tool_phase(
    transaction: &Transaction<'_>,
    activity: &ConversationActivityFact,
) -> Result<Option<u64>, LedgerError> {
    let ConversationActivityFact::ToolActivity { scope, activity } = activity else {
        return Ok(None);
    };
    let phase = serde_json::to_value(&activity.phase).map_err(storage_error)?;
    let recorded = transaction
        .query_row(
            "SELECT cursor, json_extract(payload, '$.activity.phase') FROM conversation_activity_facts WHERE conversation_id = ?1 AND run_id = ?2 AND json_extract(payload, '$.type') = 'toolActivity' AND json_extract(payload, '$.turnId') = ?3 AND json_extract(payload, '$.activity.toolUseId') = ?4 ORDER BY cursor DESC LIMIT 1",
            rusqlite::params![scope.conversation_id, scope.run_id, scope.turn_id, activity.tool_use_id],
            |row| Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(storage_error)?;
    Ok(recorded
        .filter(|(_, recorded)| phase.as_str() == Some(recorded.as_str()))
        .map(|(cursor, _)| cursor))
}

pub(super) fn scope(fact: &ConversationActivityFact) -> &gent_types::ConversationActivityScope {
    fact.scope()
}
