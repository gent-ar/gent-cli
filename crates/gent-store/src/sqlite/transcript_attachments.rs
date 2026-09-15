use gent_ports::LedgerError;
use gent_types::{
    AgentChatProjectionEvent, AttachmentReference, NormalizedTranscriptEvent,
    NormalizedTranscriptKind,
};
use rusqlite::Connection;
use serde_json::Value;

use super::queries::storage_error;

pub(super) fn attach_to_events(
    connection: &Connection,
    events: &mut [NormalizedTranscriptEvent],
) -> Result<(), LedgerError> {
    for event in events
        .iter_mut()
        .filter(|event| event.kind == NormalizedTranscriptKind::UserMessage)
    {
        event.attachments = turn_references(connection, &event.turn_id)?;
    }
    Ok(())
}

pub(super) fn attach_to_projection(
    connection: &Connection,
    events: &mut [AgentChatProjectionEvent],
) -> Result<(), LedgerError> {
    for event in events.iter_mut().filter(|event| event.kind == "transcript") {
        let Some(payload) = event.payload.as_object_mut() else {
            continue;
        };
        if payload.get("kind").and_then(Value::as_str) != Some("userMessage") {
            continue;
        }
        let Some(turn_id) = payload.get("turnId").and_then(Value::as_str) else {
            continue;
        };
        let references = turn_references(connection, turn_id)?;
        if !references.is_empty() {
            payload.insert(
                "attachments".into(),
                serde_json::to_value(references).map_err(storage_error)?,
            );
        }
    }
    Ok(())
}

fn turn_references(
    connection: &Connection,
    turn_id: &str,
) -> Result<Vec<AttachmentReference>, LedgerError> {
    let mut statement = connection
        .prepare_cached("SELECT a.attachment_id, a.display_name, a.media_type, a.byte_len FROM turn_attachments t JOIN attachments a ON a.attachment_id = t.attachment_id WHERE t.turn_id = ?1 AND a.state = 'available' ORDER BY t.rowid")
        .map_err(storage_error)?;
    statement
        .query_map([turn_id], |row| {
            Ok(AttachmentReference {
                attachment_id: row.get(0)?,
                display_name: row.get(1)?,
                media_type: row.get(2)?,
                byte_len: row.get(3)?,
            })
        })
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)
}
