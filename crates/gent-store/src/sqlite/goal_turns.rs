use gent_ports::LedgerError;
use gent_types::{
    DurableTurnPhase, GoalDispatchState, GoalTurnObservation, ProviderFailureClassification,
};
use rusqlite::{Connection, params};

use super::storage_error;

struct ObservedRow {
    ordinal: u64,
    message_id: String,
    turn_id: String,
    run_id: String,
    receipt_id: String,
    phase: String,
    dispatch: String,
    origin: Option<gent_types::AgentChatPromptOrigin>,
    held: bool,
}

pub(super) fn observe(
    connection: &Connection,
    conversation_id: &str,
    after_ordinal: u64,
) -> Result<Vec<GoalTurnObservation>, LedgerError> {
    let mut statement = connection
        .prepare(
            "SELECT o.ordinal, m.message_id, m.turn_id, m.run_id, COALESCE(r.receipt_id, ''), t.phase, COALESCE(d.state, 'settled'), te.origin_json, \
             EXISTS (SELECT 1 FROM conversation_activity_facts f WHERE f.conversation_id = m.conversation_id AND f.run_id = m.run_id AND json_extract(f.payload, '$.type') = 'promptHeld' AND json_extract(f.payload, '$.messageId') = m.message_id) \
             AND NOT EXISTS (SELECT 1 FROM conversation_activity_facts f WHERE f.conversation_id = m.conversation_id AND f.run_id = m.run_id AND json_extract(f.payload, '$.type') IN ('promptReleased', 'promptCanceled') AND json_extract(f.payload, '$.messageId') = m.message_id) \
             FROM conversation_message_ordinals o \
             JOIN conversation_messages m ON m.message_id = o.message_id \
             JOIN turns t ON t.turn_id = m.turn_id \
             LEFT JOIN agent_chat_prompt_dispatches d ON d.message_id = m.message_id \
             LEFT JOIN agent_chat_prompt_receipts p ON p.message_id = m.message_id \
             LEFT JOIN receipts r ON r.idempotency_key = p.idempotency_key \
             LEFT JOIN agent_chat_transcript_events te ON te.event_id = 'user:' || m.message_id \
             WHERE o.conversation_id = ?1 AND o.ordinal > ?2 ORDER BY o.ordinal ASC LIMIT ?3",
        )
        .map_err(storage_error)?;
    let rows = statement
        .query_map(
            params![
                conversation_id,
                after_ordinal,
                i64::try_from(gent_ports::MAX_GOAL_TURN_OBSERVATIONS).unwrap_or(i64::MAX)
            ],
            |row| {
                Ok(ObservedRow {
                    ordinal: row.get(0)?,
                    message_id: row.get(1)?,
                    turn_id: row.get(2)?,
                    run_id: row.get(3)?,
                    receipt_id: row.get(4)?,
                    phase: row.get(5)?,
                    dispatch: row.get(6)?,
                    origin: super::decode_origin(row.get(7)?)?,
                    held: row.get(8)?,
                })
            },
        )
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)?;
    rows.into_iter()
        .map(|row| {
            let phase = parse_phase(&row.phase)?;
            let (tokens, tool_calls, failure) = if phase.is_terminal() {
                usage(connection, conversation_id, &row.run_id, &row.turn_id)?
            } else {
                (0, 0, None)
            };
            Ok(GoalTurnObservation {
                ordinal: row.ordinal,
                continuation_of: row
                    .origin
                    .as_ref()
                    .and_then(|origin| origin.goal_id())
                    .map(str::to_owned),
                message_id: row.message_id,
                turn_id: row.turn_id,
                run_id: row.run_id,
                receipt_id: row.receipt_id,
                phase,
                dispatch: parse_dispatch(&row.dispatch),
                held: row.held,
                tokens,
                tool_calls,
                failure,
            })
        })
        .collect()
}

fn usage(
    connection: &Connection,
    conversation_id: &str,
    run_id: &str,
    turn_id: &str,
) -> Result<(u64, u32, Option<ProviderFailureClassification>), LedgerError> {
    let (tokens, tool_calls) = connection
        .query_row(
            "SELECT \
             COALESCE(SUM(CASE WHEN json_extract(payload, '$.type') = 'tokenUsage' THEN json_extract(payload, '$.usage.inputTokens') + json_extract(payload, '$.usage.outputTokens') + json_extract(payload, '$.usage.cacheReadTokens') + json_extract(payload, '$.usage.cacheCreationTokens') END), 0), \
             COUNT(DISTINCT CASE WHEN json_extract(payload, '$.type') = 'toolActivity' THEN json_extract(payload, '$.activity.toolUseId') END) \
             FROM conversation_activity_facts WHERE conversation_id = ?1 AND run_id = ?2 AND json_extract(payload, '$.turnId') = ?3",
            params![conversation_id, run_id, turn_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, u32>(1)?)),
        )
        .map_err(storage_error)?;
    let failure = connection
        .query_row(
            "SELECT MAX(json_extract(payload, '$.lifecycle.event.classification')) FROM agent_chat_projection_events WHERE conversation_id = ?1 AND kind = 'lifecycle' AND json_extract(payload, '$.turnId') = ?2 AND json_extract(payload, '$.lifecycle.event.type') = 'providerFailure'",
            params![conversation_id, turn_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .map_err(storage_error)?;
    Ok((
        u64::try_from(tokens).unwrap_or_default(),
        tool_calls,
        failure.as_deref().map(parse_failure),
    ))
}

fn parse_phase(value: &str) -> Result<DurableTurnPhase, LedgerError> {
    serde_json::from_value(serde_json::Value::String(value.into()))
        .map_err(|_| LedgerError::Invariant("stored turn phase is invalid".into()))
}

fn parse_dispatch(value: &str) -> GoalDispatchState {
    match value {
        "awaiting_readiness" | "provisioning" => GoalDispatchState::AwaitingReadiness,
        "pending" => GoalDispatchState::Pending,
        "claimed" | "launching" | "started" => GoalDispatchState::InFlight,
        "unprovable" => GoalDispatchState::Unprovable,
        _ => GoalDispatchState::Settled,
    }
}

fn parse_failure(value: &str) -> ProviderFailureClassification {
    serde_json::from_value(serde_json::Value::String(value.into()))
        .unwrap_or(ProviderFailureClassification::Provider)
}
