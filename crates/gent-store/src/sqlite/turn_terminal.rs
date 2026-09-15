use gent_ports::LedgerError;
use gent_types::{
    ConversationActivityFact, ConversationActivityScope, DurableTurnPhase, Event, HostEpoch,
    ReceiptId, TurnPhase, TurnTerminalCause,
};
use rusqlite::{OptionalExtension, Transaction, params};

use super::{conversation_activity_ledger, pending_permission_ledger, queries};

pub(crate) fn settle(
    transaction: &Transaction<'_>,
    turn_id: &str,
    host_epoch: HostEpoch,
    requested: DurableTurnPhase,
) -> Result<bool, LedgerError> {
    activity_phase(requested)?;
    let (conversation_id, run_id) = scope(transaction, turn_id)?;
    let recorded = recorded_phase(transaction, &conversation_id, &run_id, turn_id)?;
    let phase = recorded.unwrap_or(requested);
    let changed = transaction
        .execute(
            "UPDATE turns SET phase = ?1 WHERE turn_id = ?2 AND phase IN ('active', 'waitingPermission', 'waitingQuestion')",
            params![phase_name(phase), turn_id],
        )
        .map_err(queries::storage_error)?;
    if changed == 0 {
        return Ok(false);
    }
    pending_permission_ledger::settle_for_turn(transaction, turn_id)?;
    if recorded.is_none() {
        if phase != DurableTurnPhase::Completed {
            super::transcript_settlement::supersede_unfinished_reply(transaction, turn_id)?;
        }
        append(
            transaction,
            &conversation_id,
            &run_id,
            turn_id,
            host_epoch,
            phase,
        )?;
    }
    Ok(true)
}

fn scope(transaction: &Transaction<'_>, turn_id: &str) -> Result<(String, String), LedgerError> {
    transaction
        .query_row(
            "SELECT conversation_id, run_id FROM turns WHERE turn_id = ?1",
            params![turn_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(queries::storage_error)
}

fn recorded_phase(
    transaction: &Transaction<'_>,
    conversation_id: &str,
    run_id: &str,
    turn_id: &str,
) -> Result<Option<DurableTurnPhase>, LedgerError> {
    let recorded = transaction
        .query_row(
            "SELECT json_extract(payload, '$.phase') FROM conversation_activity_facts WHERE conversation_id = ?1 AND run_id = ?2 AND json_extract(payload, '$.type') = 'terminal' AND json_extract(payload, '$.turnId') = ?3 LIMIT 1",
            params![conversation_id, run_id, turn_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(queries::storage_error)?;
    recorded.map_or(Ok(None), |value| match value.as_str() {
        "ready" => Ok(Some(DurableTurnPhase::Completed)),
        "interrupted" => Ok(Some(DurableTurnPhase::Interrupted)),
        "cancelled" => Ok(Some(DurableTurnPhase::Cancelled)),
        "failed" => Ok(Some(DurableTurnPhase::Failed)),
        _ => Err(LedgerError::Invariant(
            "terminal activity carries a non-terminal phase".into(),
        )),
    })
}

fn append(
    transaction: &Transaction<'_>,
    conversation_id: &str,
    run_id: &str,
    turn_id: &str,
    host_epoch: HostEpoch,
    phase: DurableTurnPhase,
) -> Result<(), LedgerError> {
    let fact = ConversationActivityFact::Terminal {
        scope: ConversationActivityScope {
            conversation_id: conversation_id.into(),
            run_id: run_id.into(),
            turn_id: turn_id.into(),
            host_epoch,
            cursor: 0,
        },
        phase: activity_phase(phase)?,
        cause: cause(transaction, turn_id, phase)?,
    };
    let event = queries::append_event(
        transaction,
        &Event {
            cursor: 0,
            event_id: format!("agent-chat-terminal:{turn_id}:{}", phase_name(phase)),
            receipt_id: ReceiptId(format!("agentChatTerminal:{turn_id}")),
            host_epoch,
            kind: "providerActivity".into(),
            payload: serde_json::json!({
                "conversationId": conversation_id,
                "runId": run_id,
                "turnId": turn_id,
                "activity": fact,
            }),
        },
    )?;
    conversation_activity_ledger::append(
        transaction,
        &gent_core::with_activity_cursor(fact, event.cursor),
    )
}

pub(crate) const STEER_INTERRUPT_KEY_PREFIX: &str = "agent-chat-steer-interrupt:";

fn cause(
    transaction: &Transaction<'_>,
    turn_id: &str,
    phase: DurableTurnPhase,
) -> Result<Option<TurnTerminalCause>, LedgerError> {
    if phase == DurableTurnPhase::Failed && provider_session_unavailable(transaction, turn_id)? {
        return Ok(Some(TurnTerminalCause::ProviderSessionUnavailable));
    }
    if phase == DurableTurnPhase::Failed {
        return transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM conversation_messages m JOIN agent_chat_prompt_dispatches d ON d.message_id = m.message_id WHERE m.turn_id = ?1 AND d.state IN ('launching', 'unprovable'))",
                [turn_id],
                |row| row.get::<_, bool>(0),
            )
            .map(|unprovable| unprovable.then_some(TurnTerminalCause::DeliveryUnprovable))
            .map_err(queries::storage_error);
    }
    if phase != DurableTurnPhase::Interrupted {
        return Ok(None);
    }
    transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM receipts WHERE idempotency_key = ?1)",
            [format!("{STEER_INTERRUPT_KEY_PREFIX}{turn_id}")],
            |row| row.get::<_, bool>(0),
        )
        .map(|steered| steered.then_some(TurnTerminalCause::Steered))
        .map_err(queries::storage_error)
}

fn provider_session_unavailable(
    transaction: &Transaction<'_>,
    turn_id: &str,
) -> Result<bool, LedgerError> {
    transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_chat_projection_events WHERE conversation_id = (SELECT conversation_id FROM turns WHERE turn_id = ?1) AND kind = 'lifecycle' AND json_extract(payload, '$.turnId') = ?1 AND json_extract(payload, '$.lifecycle.event.type') = 'providerFailure' AND json_extract(payload, '$.lifecycle.event.classification') = 'sessionUnavailable')",
            [turn_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(queries::storage_error)
}

fn activity_phase(phase: DurableTurnPhase) -> Result<TurnPhase, LedgerError> {
    match phase {
        DurableTurnPhase::Completed => Ok(TurnPhase::Ready),
        DurableTurnPhase::Interrupted => Ok(TurnPhase::Interrupted),
        DurableTurnPhase::Cancelled => Ok(TurnPhase::Cancelled),
        DurableTurnPhase::Failed => Ok(TurnPhase::Failed),
        DurableTurnPhase::Active
        | DurableTurnPhase::WaitingPermission
        | DurableTurnPhase::WaitingQuestion => Err(LedgerError::Invariant(
            "turn settlement phase is not terminal".into(),
        )),
    }
}

const fn phase_name(phase: DurableTurnPhase) -> &'static str {
    match phase {
        DurableTurnPhase::Completed => "completed",
        DurableTurnPhase::Interrupted => "interrupted",
        DurableTurnPhase::Cancelled => "cancelled",
        DurableTurnPhase::Failed => "failed",
        DurableTurnPhase::Active => "active",
        DurableTurnPhase::WaitingPermission => "waitingPermission",
        DurableTurnPhase::WaitingQuestion => "waitingQuestion",
    }
}
