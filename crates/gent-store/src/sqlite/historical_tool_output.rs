use gent_ports::LedgerError;
use gent_types::{
    AgentChatProjectionEvent, ConversationActivityFact, ConversationActivityScope,
    NormalizedProviderEvent, NormalizedSessionBatch, NormalizedSessionLifecycle,
};
use rusqlite::{Connection, OptionalExtension};

use super::queries::storage_error;

pub(super) const ACTIVITY_PAGE: &str = "SELECT cursor, payload, lifecycle_event_id FROM (SELECT cursor, payload, NULL AS lifecycle_event_id FROM conversation_activity_facts WHERE conversation_id = ?1 AND run_id = ?2 UNION ALL SELECT f.cursor AS cursor, b.payload, b.lifecycle_event_id FROM run_lifecycle_facts f JOIN normalized_session_batches b ON b.lifecycle_event_id = f.event_id WHERE f.run_id = ?2 AND f.cursor > ?3 AND json_extract(b.payload, '$.conversationId') = ?1 AND +b.activity_event_id IS NULL AND +b.activity_cursor IS NULL AND json_extract(b.payload, '$.lifecycle.event.type') = 'toolOutputDelta') WHERE cursor > ?3 ORDER BY cursor LIMIT ?4";

pub(super) fn read(
    connection: &Connection,
    event_id: &str,
) -> Result<Option<ConversationActivityFact>, LedgerError> {
    let recorded = connection.query_row(
        "SELECT b.payload, b.lifecycle_cursor, e.payload, e.host_epoch FROM normalized_session_batches b JOIN events e ON e.cursor = b.lifecycle_cursor AND e.event_id = b.lifecycle_event_id AND e.kind = 'normalizedSessionLifecycle' WHERE b.lifecycle_event_id = ?1 AND b.activity_event_id IS NULL AND b.activity_cursor IS NULL",
        [event_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?, row.get::<_, String>(2)?, row.get::<_, u64>(3)?))
    ).optional().map_err(storage_error)?;
    let Some((batch, cursor, event, epoch)) = recorded else {
        return Ok(None);
    };
    let batch: NormalizedSessionBatch = serde_json::from_str(&batch).map_err(storage_error)?;
    let event: serde_json::Value = serde_json::from_str(&event).map_err(storage_error)?;
    if batch.lifecycle_event_id != event_id
        || batch.host_epoch.0 != epoch
        || batch.activity.is_some()
        || event
            != serde_json::json!({"conversationId": batch.conversation_id, "runId": batch.run_id, "turnId": batch.turn_id, "lifecycle": batch.lifecycle})
    {
        return Err(LedgerError::Invariant(
            "historical output source does not match its normalized batch".into(),
        ));
    }
    let NormalizedSessionLifecycle::Event {
        event:
            NormalizedProviderEvent::ToolOutputDelta {
                tool_use_id,
                text,
                is_partial,
            },
    } = batch.lifecycle
    else {
        return Ok(None);
    };
    Ok(Some(ConversationActivityFact::ToolOutput {
        scope: ConversationActivityScope {
            conversation_id: batch.conversation_id,
            run_id: batch.run_id,
            turn_id: batch.turn_id,
            host_epoch: batch.host_epoch,
            cursor,
        },
        tool_use_id,
        text: gent_types::bounded_text(&text, gent_types::MAX_TRANSCRIPT_TEXT_BYTES).into_owned(),
        is_partial,
        source_lifecycle_event_id: Some(event_id.into()),
    }))
}

pub(super) fn project(
    connection: &Connection,
    event: &mut AgentChatProjectionEvent,
) -> Result<(), LedgerError> {
    if event.kind != "lifecycle"
        || event
            .payload
            .pointer("/lifecycle/event/type")
            .and_then(serde_json::Value::as_str)
            != Some("toolOutputDelta")
    {
        return Ok(());
    }
    let Some(source) = event.source_event_id.strip_prefix("lifecycle:") else {
        return Ok(());
    };
    let Some(activity) = read(connection, source)? else {
        return Ok(());
    };
    let scope = activity.scope();
    if event
        .payload
        .get("conversationId")
        .and_then(serde_json::Value::as_str)
        != Some(scope.conversation_id.as_str())
        || event
            .payload
            .get("runId")
            .and_then(serde_json::Value::as_str)
            != Some(scope.run_id.as_str())
        || event
            .payload
            .get("turnId")
            .and_then(serde_json::Value::as_str)
            != Some(scope.turn_id.as_str())
    {
        return Err(LedgerError::Invariant(
            "historical output projection scope does not match its source".into(),
        ));
    }
    event.kind = "activity".into();
    event.payload = serde_json::json!({"conversationId": scope.conversation_id, "runId": scope.run_id, "turnId": scope.turn_id, "activity": activity, "derivedFromLifecycleEventId": source});
    Ok(())
}

#[cfg(test)]
#[path = "historical_tool_output_plan_tests.rs"]
mod plan_tests;
