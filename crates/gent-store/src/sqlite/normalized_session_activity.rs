//! Activity portions of an atomic normalized-session write.

use gent_ports::LedgerError;
use gent_types::{ConversationActivityFact, Event, NormalizedSessionBatch, ReceiptId};
use rusqlite::Transaction;

use super::{conversation_activity_ledger, queries::append_event};

pub(super) fn append(
    transaction: &Transaction<'_>,
    batch: &NormalizedSessionBatch,
) -> Result<Option<u64>, LedgerError> {
    let (Some(event_id), Some(activity)) = (&batch.activity_event_id, &batch.activity) else {
        return Ok(None);
    };
    append_event(
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
    )
    .map(|event| Some(event.cursor))
}

pub(super) fn apply(
    transaction: &Transaction<'_>,
    batch: &NormalizedSessionBatch,
    cursor: Option<u64>,
) -> Result<(), LedgerError> {
    let (Some(activity), Some(cursor)) = (&batch.activity, cursor) else {
        return Ok(());
    };
    conversation_activity_ledger::append(
        transaction,
        &gent_core::with_activity_cursor(activity.clone(), cursor),
    )
}

pub(super) fn scope(fact: &ConversationActivityFact) -> &gent_types::ConversationActivityScope {
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
