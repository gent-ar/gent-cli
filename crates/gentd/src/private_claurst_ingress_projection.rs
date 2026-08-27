use gent_ports::{
    ClaurstFactValue, ClaurstNormalizedFact, Ledger, NormalizedSessionBatchLedger,
    PrivateClaurstBridge, RunCheckpointLedger, RunLifecycleFactLedger, TranscriptLedger,
};
use gent_runtime::{ProviderLifecycleEffect, RuntimeError};
use gent_types::{
    HostEpoch, NormalizedProviderEvent, NormalizedSessionBatch, NormalizedSessionLifecycle,
    NormalizedTranscriptAppend, NormalizedTranscriptKind,
};

use super::validation::event_id;
use super::{BoundSource, PrivateClaurstIngress};

impl<L, B> PrivateClaurstIngress<L, B>
where
    L: Clone
        + std::fmt::Debug
        + Ledger
        + gent_ports::GoalLedger
        + RunCheckpointLedger
        + RunLifecycleFactLedger
        + NormalizedSessionBatchLedger
        + TranscriptLedger,
    B: PrivateClaurstBridge,
{
    pub(super) fn record_fact(
        &self,
        state: &BoundSource,
        fact: &ClaurstNormalizedFact,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        let binding = &state.binding;
        let (Some(conversation_id), Some(turn_id)) =
            (state.conversation_id.as_ref(), state.turn_id.as_ref())
        else {
            return self.record_lifecycle_fact(binding, fact, host_epoch);
        };
        let lifecycle = match &fact.value {
            ClaurstFactValue::Event(event) => NormalizedSessionLifecycle::Event {
                event: event.clone(),
            },
            ClaurstFactValue::Lifecycle(signal) => NormalizedSessionLifecycle::Signal {
                signal: signal.clone(),
            },
        };
        let transcript = match &fact.value {
            ClaurstFactValue::Event(NormalizedProviderEvent::Output { text, is_partial }) => {
                Some(transcript(
                    binding,
                    turn_id,
                    fact.cursor,
                    NormalizedTranscriptKind::AssistantMessage,
                    text,
                    *is_partial,
                ))
            }
            ClaurstFactValue::Event(NormalizedProviderEvent::Thinking { text, is_partial }) => {
                Some(transcript(
                    binding,
                    turn_id,
                    fact.cursor,
                    NormalizedTranscriptKind::Thinking,
                    text,
                    *is_partial,
                ))
            }
            _ => None,
        };
        let activity = crate::public_driver_runtime::session::activity_for_lifecycle(
            &conversation_id.0,
            &binding.run_id,
            turn_id,
            host_epoch,
            &lifecycle,
        );
        self.ledger
            .append_normalized_session_batch(&NormalizedSessionBatch {
                coordinator_id: self.coordinator_id.clone(),
                conversation_id: conversation_id.0.clone(),
                run_id: binding.run_id.clone(),
                turn_id: turn_id.clone(),
                host_epoch,
                lifecycle_event_id: event_id(&binding.source_id, &format!("fact-{}", fact.cursor)),
                lifecycle,
                transcript,
                activity_event_id: activity
                    .as_ref()
                    .map(|_| event_id(&binding.source_id, &format!("activity-{}", fact.cursor))),
                activity,
            })
            .map_err(RuntimeError::Ledger)?;
        Ok(())
    }

    fn record_lifecycle_fact(
        &self,
        binding: &gent_ports::ClaurstSessionBinding,
        fact: &ClaurstNormalizedFact,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        let effect = match &fact.value {
            ClaurstFactValue::Event(event) => ProviderLifecycleEffect::Normalized(event.clone()),
            ClaurstFactValue::Lifecycle(signal) => {
                ProviderLifecycleEffect::Lifecycle(signal.clone())
            }
        };
        self.lifecycle.record(
            event_id(&binding.source_id, &format!("fact-{}", fact.cursor)),
            &binding.run_id,
            &self.coordinator_id,
            host_epoch,
            effect,
        )?;
        Ok(())
    }
}

fn transcript(
    binding: &gent_ports::ClaurstSessionBinding,
    turn_id: &str,
    cursor: u64,
    kind: NormalizedTranscriptKind,
    text: &str,
    is_partial: bool,
) -> NormalizedTranscriptAppend {
    NormalizedTranscriptAppend {
        event_id: event_id(&binding.source_id, &format!("transcript-{cursor}")),
        run_id: binding.run_id.clone(),
        turn_id: turn_id.into(),
        kind,
        text: text.into(),
        is_partial,
    }
}
