use gent_protocol::{
    AgentChatProjectionDelta, ProjectionCursor, ProjectionRecordKind, UnreadableProjectionRecord,
};
use gent_types::{
    AgentChatConversationSummary, AgentChatFailureCode, AgentChatProjectionEvent,
    ConversationActivityFact, NormalizedSessionLifecycle, NormalizedTranscriptEvent,
};

pub(super) type SummaryRead<'a> = &'a dyn Fn() -> Result<AgentChatConversationSummary, String>;

pub(super) fn deltas(
    events: Vec<AgentChatProjectionEvent>,
    summary: SummaryRead<'_>,
) -> Result<Vec<AgentChatProjectionDelta>, String> {
    events
        .into_iter()
        .map(|event| delta(event, summary))
        .collect()
}

pub(super) fn readable_and_unreadable<T>(
    events: Vec<AgentChatProjectionEvent>,
    record_kind: ProjectionRecordKind,
    decode: fn(serde_json::Value) -> Result<T, String>,
) -> (Vec<T>, Vec<UnreadableProjectionRecord>) {
    let mut readable = Vec::new();
    let mut unreadable = Vec::new();
    for event in events {
        let cursor = event.cursor;
        match decode(event.payload) {
            Ok(decoded) => readable.push(decoded),
            Err(error) => unreadable.push(unreadable_record(cursor, record_kind, &error)),
        }
    }
    (readable, unreadable)
}

fn unreadable_record(
    cursor: u64,
    record_kind: ProjectionRecordKind,
    error: &str,
) -> UnreadableProjectionRecord {
    eprintln!("projection record {cursor} ({record_kind:?}) is unreadable: {error}");
    UnreadableProjectionRecord {
        cursor: ProjectionCursor { value: cursor },
        record_kind,
    }
}

pub(super) fn delta(
    event: AgentChatProjectionEvent,
    summary: SummaryRead<'_>,
) -> Result<AgentChatProjectionDelta, String> {
    let cursor = ProjectionCursor {
        value: event.cursor,
    };
    let record_kind = record_kind(&event.kind);
    let decoded = match record_kind {
        ProjectionRecordKind::Transcript => {
            transcript_event(event.payload).map(|transcript| AgentChatProjectionDelta::Transcript {
                cursor: cursor.clone(),
                event: Box::new(transcript),
            })
        }
        ProjectionRecordKind::Activity => {
            activity_fact(event.payload).map(|fact| AgentChatProjectionDelta::Activity {
                cursor: cursor.clone(),
                fact: Box::new(fact),
            })
        }
        ProjectionRecordKind::Lifecycle => {
            lifecycle(event.payload).map(|lifecycle| AgentChatProjectionDelta::Lifecycle {
                cursor: cursor.clone(),
                lifecycle,
            })
        }
        ProjectionRecordKind::Summary => match summary_run(&event.payload) {
            Ok(run_id) => {
                return Ok(AgentChatProjectionDelta::Summary {
                    cursor,
                    run_id,
                    summary: Box::new(summary()?),
                });
            }
            Err(error) => Err(error),
        },
        ProjectionRecordKind::Unrecognized => {
            Err(format!("unknown projection event kind {}", event.kind))
        }
    };
    Ok(decoded.unwrap_or_else(|error| {
        let record = unreadable_record(cursor.value, record_kind, &error);
        AgentChatProjectionDelta::UnreadableRecord {
            cursor: record.cursor,
            record_kind: record.record_kind,
        }
    }))
}

fn record_kind(kind: &str) -> ProjectionRecordKind {
    match kind {
        "transcript" => ProjectionRecordKind::Transcript,
        "activity" => ProjectionRecordKind::Activity,
        "lifecycle" => ProjectionRecordKind::Lifecycle,
        "summary" => ProjectionRecordKind::Summary,
        _ => ProjectionRecordKind::Unrecognized,
    }
}

fn lifecycle(payload: serde_json::Value) -> Result<NormalizedSessionLifecycle, String> {
    serde_json::from_value(
        payload
            .get("lifecycle")
            .cloned()
            .ok_or_else(|| "projection lifecycle has no fact".to_owned())?,
    )
    .map_err(|error| error.to_string())
}

fn summary_run(payload: &serde_json::Value) -> Result<String, String> {
    payload
        .get("runId")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "projection summary has no run".to_owned())
}

pub(super) fn transcript_event(
    payload: serde_json::Value,
) -> Result<NormalizedTranscriptEvent, String> {
    let mut event: NormalizedTranscriptEvent =
        serde_json::from_value(normalize_projection_transcript(payload))
            .map_err(|error| error.to_string())?;
    event.failure_code = AgentChatFailureCode::from_event_id(&event.event_id);
    event.resets_at = AgentChatFailureCode::resets_at_from_event_id(&event.event_id);
    Ok(event)
}

pub(super) fn activity_fact(
    payload: serde_json::Value,
) -> Result<ConversationActivityFact, String> {
    serde_json::from_value(
        payload
            .get("activity")
            .cloned()
            .ok_or_else(|| "projection activity has no fact".to_owned())?,
    )
    .map_err(|error| error.to_string())
}

pub(super) fn normalize_projection_transcript(mut payload: serde_json::Value) -> serde_json::Value {
    if let Some(partial) = payload.get_mut("isPartial") {
        if let Some(value) = partial.as_i64() {
            *partial = serde_json::Value::Bool(value != 0);
        }
    }
    payload
}
