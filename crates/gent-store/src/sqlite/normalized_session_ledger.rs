//! Atomic persistence for one already-normalized provider session fact.
use gent_ports::{IngressMode, LedgerError, NormalizedSessionBatchLedger, RunLease};
use gent_types::{
    ConversationActivityFact, Event, NormalizedProviderEvent, NormalizedSessionBatch,
    NormalizedSessionBatchResult, ReceiptId,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use super::{
    SqliteLedger,
    epoch::require_epoch,
    normalized_session_projection::{apply_activity, apply_lifecycle},
    queries::{
        append_event, find_event, find_run_lease, find_run_session_binding, host_ingress,
        storage_error,
    },
};

impl NormalizedSessionBatchLedger for SqliteLedger {
    fn append_normalized_session_batch(
        &self,
        batch: &NormalizedSessionBatch,
    ) -> Result<NormalizedSessionBatchResult, LedgerError> {
        validate(batch)?;
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        if let Some(result) = existing(&transaction, batch)? {
            return Ok(result);
        }
        require_owner(&transaction, batch)?;
        reject_source_collisions(&transaction, batch)?;
        let lifecycle = append_event(&transaction, &lifecycle_event(batch))?;
        apply_lifecycle(&transaction, batch, lifecycle.cursor)?;
        let transcript_cursor = append_transcript(&transaction, batch)?;
        let activity_cursor = append_activity(&transaction, batch)?;
        apply_activity(&transaction, batch, activity_cursor)?;
        let result = NormalizedSessionBatchResult {
            lifecycle_cursor: lifecycle.cursor,
            transcript_cursor,
            activity_cursor,
        };
        transaction
            .execute(
                "INSERT INTO normalized_session_batches (lifecycle_event_id, payload, lifecycle_cursor, transcript_event_id, transcript_cursor, activity_event_id, activity_cursor) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![batch.lifecycle_event_id, fingerprint(batch)?, result.lifecycle_cursor, batch.transcript.as_ref().map(|item| &item.event_id), result.transcript_cursor, batch.activity_event_id, result.activity_cursor],
            )
            .map_err(storage_error)?;
        transaction.commit().map_err(storage_error)?;
        Ok(result)
    }
}
fn validate(batch: &NormalizedSessionBatch) -> Result<(), LedgerError> {
    if [
        &batch.coordinator_id,
        &batch.conversation_id,
        &batch.run_id,
        &batch.turn_id,
        &batch.lifecycle_event_id,
    ]
    .iter()
    .any(|value| value.trim().is_empty())
        || batch.host_epoch.0 == 0
        || (batch.activity_event_id.is_some() != batch.activity.is_some())
    {
        return Err(LedgerError::Invariant(
            "normalized session batch identity is invalid".into(),
        ));
    }
    if let Some(transcript) = &batch.transcript {
        if transcript.event_id.trim().is_empty()
            || transcript.turn_id != batch.turn_id
            || transcript.run_id != batch.run_id
            || transcript.text.contains('\0')
            || transcript.text.len() > 64 * 1024
        {
            return Err(LedgerError::Invariant(
                "normalized session transcript does not match its batch".into(),
            ));
        }
    }
    if let Some(activity) = &batch.activity {
        let scope = activity_scope(activity);
        if scope.conversation_id != batch.conversation_id
            || scope.run_id != batch.run_id
            || scope.turn_id != batch.turn_id
            || scope.host_epoch != batch.host_epoch
            || scope.cursor != 0
        {
            return Err(LedgerError::Invariant(
                "normalized session activity does not match its batch".into(),
            ));
        }
    }
    if let gent_types::NormalizedSessionLifecycle::Event {
        event:
            NormalizedProviderEvent::TurnStarted { turn_id }
            | NormalizedProviderEvent::TurnEnded { turn_id },
    } = &batch.lifecycle
    {
        if turn_id != &batch.turn_id {
            return Err(LedgerError::Invariant(
                "normalized session lifecycle turn does not match its batch".into(),
            ));
        }
    }
    Ok(())
}

fn existing(
    transaction: &Transaction<'_>,
    batch: &NormalizedSessionBatch,
) -> Result<Option<NormalizedSessionBatchResult>, LedgerError> {
    let row = transaction
        .query_row(
            "SELECT payload, lifecycle_cursor, transcript_event_id, transcript_cursor, activity_event_id, activity_cursor FROM normalized_session_batches WHERE lifecycle_event_id = ?1",
            [&batch.lifecycle_event_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, Option<u64>>(3)?, row.get::<_, Option<String>>(4)?, row.get::<_, Option<u64>>(5)?)),
        )
        .optional()
        .map_err(storage_error)?;
    let Some((
        payload,
        lifecycle_cursor,
        transcript_id,
        transcript_cursor,
        activity_id,
        activity_cursor,
    )) = row
    else {
        return Ok(None);
    };
    if payload != fingerprint(batch)?
        || transcript_id.as_deref() != batch.transcript.as_ref().map(|item| item.event_id.as_str())
        || activity_id.as_deref() != batch.activity_event_id.as_deref()
    {
        return Err(LedgerError::Invariant(
            "normalized session source retry conflicts with durable ownership".into(),
        ));
    }
    Ok(Some(NormalizedSessionBatchResult {
        lifecycle_cursor,
        transcript_cursor,
        activity_cursor,
    }))
}

fn require_owner(
    transaction: &Transaction<'_>,
    batch: &NormalizedSessionBatch,
) -> Result<(), LedgerError> {
    let ingress = host_ingress(transaction)?;
    require_epoch(batch.host_epoch, ingress.epoch)?;
    if ingress.mode == IngressMode::Closed {
        return Err(LedgerError::IngressClosed {
            epoch: ingress.epoch,
        });
    }
    let lease = find_run_lease(transaction, &batch.run_id)?;
    if !matches!(lease, Some(RunLease { coordinator_id, host_epoch, .. }) if coordinator_id == batch.coordinator_id && host_epoch == batch.host_epoch)
    {
        return Err(LedgerError::Invariant(
            "normalized session reporter does not own the run".into(),
        ));
    }
    if find_run_session_binding(transaction, &batch.run_id)?.is_none() {
        return Err(LedgerError::Invariant(
            "normalized session batch requires a daemon-owned session".into(),
        ));
    }
    let valid_turn = transaction
        .query_row(
            "SELECT 1 FROM turns WHERE conversation_id = ?1 AND run_id = ?2 AND turn_id = ?3",
            params![batch.conversation_id, batch.run_id, batch.turn_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(storage_error)?
        .is_some();
    valid_turn.then_some(()).ok_or_else(|| {
        LedgerError::Invariant("normalized session batch has an unknown turn hierarchy".into())
    })
}

fn reject_source_collisions(
    transaction: &Transaction<'_>,
    batch: &NormalizedSessionBatch,
) -> Result<(), LedgerError> {
    let ids = std::iter::once(batch.lifecycle_event_id.as_str())
        .chain(batch.activity_event_id.as_deref());
    for id in ids {
        if find_event(transaction, id)?.is_some() {
            return Err(LedgerError::Invariant(
                "normalized session source id is already owned".into(),
            ));
        }
    }
    if let Some(transcript) = &batch.transcript {
        let exists = transaction
            .query_row(
                "SELECT 1 FROM agent_chat_transcript_events WHERE event_id = ?1",
                [&transcript.event_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(storage_error)?
            .is_some();
        if exists {
            return Err(LedgerError::Invariant(
                "normalized session transcript id is already owned".into(),
            ));
        }
    }
    Ok(())
}

fn lifecycle_event(batch: &NormalizedSessionBatch) -> Event {
    Event {
        cursor: 0,
        event_id: batch.lifecycle_event_id.clone(),
        receipt_id: ReceiptId(format!("normalizedSession:{}", batch.run_id)),
        host_epoch: batch.host_epoch,
        kind: "normalizedSessionLifecycle".into(),
        payload: serde_json::json!({
            "conversationId": batch.conversation_id,
            "runId": batch.run_id,
            "turnId": batch.turn_id,
            "lifecycle": batch.lifecycle,
        }),
    }
}

fn append_transcript(
    transaction: &Transaction<'_>,
    batch: &NormalizedSessionBatch,
) -> Result<Option<u64>, LedgerError> {
    let Some(item) = &batch.transcript else {
        return Ok(None);
    };
    let cursor = transaction
        .query_row(
            "SELECT COALESCE(MAX(cursor), 0) + 1 FROM agent_chat_transcript_events WHERE conversation_id = ?1",
            [&batch.conversation_id],
            |row| row.get::<_, u64>(0),
        )
        .map_err(storage_error)?;
    transaction.execute("INSERT INTO agent_chat_transcript_events (conversation_id, cursor, event_id, turn_id, run_id, kind, text, is_partial) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)", params![batch.conversation_id, cursor, item.event_id, item.turn_id, item.run_id, transcript_kind(item.kind), item.text, i64::from(item.is_partial)]).map_err(storage_error)?;
    Ok(Some(cursor))
}

fn append_activity(
    transaction: &Transaction<'_>,
    batch: &NormalizedSessionBatch,
) -> Result<Option<u64>, LedgerError> {
    let (Some(event_id), Some(activity)) = (&batch.activity_event_id, &batch.activity) else {
        return Ok(None);
    };
    let event = Event {
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
    };
    append_event(transaction, &event).map(|event| Some(event.cursor))
}

fn activity_scope(fact: &ConversationActivityFact) -> &gent_types::ConversationActivityScope {
    match fact {
        ConversationActivityFact::TurnStarted { scope }
        | ConversationActivityFact::RootActivity { scope, .. }
        | ConversationActivityFact::RootPhase { scope, .. }
        | ConversationActivityFact::WorkPhase { scope, .. }
        | ConversationActivityFact::DecisionPending { scope, .. }
        | ConversationActivityFact::DecisionSettled { scope, .. }
        | ConversationActivityFact::InterruptRequested { scope }
        | ConversationActivityFact::Recovered { scope }
        | ConversationActivityFact::Terminal { scope, .. } => scope,
    }
}

fn fingerprint(batch: &NormalizedSessionBatch) -> Result<String, LedgerError> {
    serde_json::to_string(batch).map_err(storage_error)
}

const fn transcript_kind(kind: gent_types::NormalizedTranscriptKind) -> &'static str {
    match kind {
        gent_types::NormalizedTranscriptKind::UserMessage => "userMessage",
        gent_types::NormalizedTranscriptKind::AssistantMessage => "assistantMessage",
        gent_types::NormalizedTranscriptKind::ToolActivity => "toolActivity",
        gent_types::NormalizedTranscriptKind::Notice => "notice",
    }
}
