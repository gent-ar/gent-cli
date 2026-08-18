//! Pure reducer application inside the normalized-session storage transaction.

use gent_core::{
    ConversationActivityProjection, project_conversation_activity, project_lifecycle_signal,
    project_normalized_event, restore_projection, snapshot_projection,
};
use gent_ports::LedgerError;
use gent_types::{
    ConversationActivityFact, ConversationActivityScope, NormalizedSessionBatch,
    NormalizedSessionLifecycle, RunProjectionRecord,
};
use rusqlite::Transaction;

use super::{conversation_activity_ledger, projections};

pub(super) fn apply_lifecycle(
    transaction: &Transaction<'_>,
    batch: &NormalizedSessionBatch,
    cursor: u64,
) -> Result<(), LedgerError> {
    let current = projections::find(transaction, &batch.run_id)?
        .map(|record| restore_projection(&record.projection))
        .unwrap_or_default();
    let update = match &batch.lifecycle {
        NormalizedSessionLifecycle::Event { event } => {
            project_normalized_event(current, cursor, event)
        }
        NormalizedSessionLifecycle::Signal { signal } => {
            project_lifecycle_signal(current, cursor, signal)
        }
    };
    if update.applied {
        projections::save(
            transaction,
            &RunProjectionRecord {
                run_id: batch.run_id.clone(),
                host_epoch: batch.host_epoch,
                projection: snapshot_projection(&update.state),
            },
        )?;
    }
    Ok(())
}

pub(super) fn apply_activity(
    transaction: &Transaction<'_>,
    batch: &NormalizedSessionBatch,
    cursor: Option<u64>,
) -> Result<(), LedgerError> {
    let (Some(activity), Some(cursor)) = (&batch.activity, cursor) else {
        return Ok(());
    };
    let current =
        conversation_activity_ledger::find(transaction, &batch.conversation_id, &batch.run_id)?
            .map_or_else(
                || {
                    ConversationActivityProjection::new(
                        batch.conversation_id.clone(),
                        batch.run_id.clone(),
                        batch.host_epoch,
                    )
                },
                ConversationActivityProjection::from_record,
            );
    let update = project_conversation_activity(current, &with_cursor(activity.clone(), cursor));
    if update.applied {
        conversation_activity_ledger::save(transaction, &update.projection.record())?;
    }
    Ok(())
}

fn with_cursor(mut fact: ConversationActivityFact, cursor: u64) -> ConversationActivityFact {
    scope_mut(&mut fact).cursor = cursor;
    fact
}

fn scope_mut(fact: &mut ConversationActivityFact) -> &mut ConversationActivityScope {
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
