//! Common durable recording of one already-normalized Claude or Codex wire fact.
//!
//! This adapter creates one daemon-owned atomic ledger batch before a future session supervisor
//! may publish a delta. It accepts no raw provider output or provider-native session identity.

use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{Ledger, NormalizedSessionBatchLedger};
use gent_runtime::RuntimeError;
use gent_types::{
    ActivityWorkKind, ConversationActivityFact, ConversationActivityScope, HostEpoch,
    NormalizedLifecycleSignal, NormalizedProviderEvent, NormalizedSessionBatch,
    NormalizedSessionLifecycle, NormalizedTranscriptAppend, NormalizedTranscriptKind, ToolPhase,
    TurnPhase, WorkPhase,
};

use super::PublicDriversRuntime;

/// Daemon-owned identities and a provider-normalized fact for one durable recording batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NormalizedSessionFact {
    pub(crate) run_id: String,
    pub(crate) conversation_id: String,
    pub(crate) turn_id: String,
    pub(crate) host_epoch: HostEpoch,
    pub(crate) lifecycle_event_id: String,
    pub(crate) transcript_event_id: String,
    pub(crate) activity_event_id: String,
    pub(crate) fact: PublicWireFact,
}

/// Cursors allocated by the durable parts of one normalized session recording.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NormalizedSessionRecord {
    pub(crate) lifecycle_cursor: u64,
    pub(crate) transcript_cursor: Option<u64>,
    pub(crate) activity_cursor: Option<u64>,
    pub(crate) terminal_signal: bool,
}

impl<L, D, R> PublicDriversRuntime<L, D, R>
where
    L: Clone
        + Ledger
        + gent_ports::RunLifecycleFactLedger
        + gent_ports::ConversationActivityLedger
        + gent_ports::TranscriptLedger
        + gent_ports::AgentChatPromptDispatchLedger
        + NormalizedSessionBatchLedger,
    D: gent_ports::PublicProviderRunner + Clone,
    R: gent_ports::PublicProviderResolver,
{
    /// Atomically persists lifecycle, transcript, and activity records for a normalized provider fact.
    ///
    /// It intentionally does not settle a prompt: the lifecycle owner must observe a terminal
    /// signal and prove its process/session is drained before calling the existing settlement API.
    pub(crate) fn record_normalized_session(
        &self,
        coordinator_id: &str,
        input: &NormalizedSessionFact,
    ) -> Result<NormalizedSessionRecord, RuntimeError> {
        validate(input)?;
        let record = self
            .ledger
            .append_normalized_session_batch(&batch(coordinator_id, input)?)?;
        Ok(NormalizedSessionRecord {
            lifecycle_cursor: record.lifecycle_cursor,
            transcript_cursor: record.transcript_cursor,
            activity_cursor: record.activity_cursor,
            terminal_signal: terminal(&input.fact),
        })
    }
}

fn batch(
    coordinator_id: &str,
    input: &NormalizedSessionFact,
) -> Result<NormalizedSessionBatch, RuntimeError> {
    let lifecycle = match &input.fact {
        PublicWireFact::Event(event) => NormalizedSessionLifecycle::Event {
            event: event.clone(),
        },
        PublicWireFact::Lifecycle(signal) => NormalizedSessionLifecycle::Signal {
            signal: signal.clone(),
        },
        PublicWireFact::SessionStarted { .. } => {
            return Err(invariant(
                "provider session binding must precede normalized session recording",
            ));
        }
        PublicWireFact::Compaction(_) => {
            return Err(invariant(
                "compaction requires its dedicated private ingress",
            ));
        }
    };
    let transcript = output(&input.fact).map(|(text, is_partial)| NormalizedTranscriptAppend {
        event_id: input.transcript_event_id.clone(),
        run_id: input.run_id.clone(),
        turn_id: input.turn_id.clone(),
        kind: NormalizedTranscriptKind::AssistantMessage,
        text,
        is_partial,
    });
    let activity = activity(input);
    Ok(NormalizedSessionBatch {
        coordinator_id: coordinator_id.into(),
        conversation_id: input.conversation_id.clone(),
        run_id: input.run_id.clone(),
        turn_id: input.turn_id.clone(),
        host_epoch: input.host_epoch,
        lifecycle_event_id: input.lifecycle_event_id.clone(),
        lifecycle,
        transcript,
        activity_event_id: activity.as_ref().map(|_| input.activity_event_id.clone()),
        activity,
    })
}

fn validate(input: &NormalizedSessionFact) -> Result<(), RuntimeError> {
    if [
        &input.run_id,
        &input.conversation_id,
        &input.turn_id,
        &input.lifecycle_event_id,
        &input.transcript_event_id,
        &input.activity_event_id,
    ]
    .iter()
    .any(|value| value.trim().is_empty())
    {
        return Err(invariant(
            "normalized session recording identity is required",
        ));
    }
    if matches!(input.fact, PublicWireFact::Compaction(_)) {
        return Err(invariant(
            "compaction requires its dedicated private ingress",
        ));
    }
    Ok(())
}

fn output(fact: &PublicWireFact) -> Option<(String, bool)> {
    match fact {
        PublicWireFact::Event(NormalizedProviderEvent::Output { text, is_partial }) => {
            Some((text.clone(), *is_partial))
        }
        _ => None,
    }
}

fn activity(input: &NormalizedSessionFact) -> Option<ConversationActivityFact> {
    let scope = || ConversationActivityScope {
        conversation_id: input.conversation_id.clone(),
        run_id: input.run_id.clone(),
        turn_id: input.turn_id.clone(),
        host_epoch: input.host_epoch,
        cursor: 0,
    };
    match &input.fact {
        PublicWireFact::Event(NormalizedProviderEvent::TurnStarted { .. }) => {
            Some(ConversationActivityFact::TurnStarted { scope: scope() })
        }
        PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootActivity { activity }) => {
            Some(ConversationActivityFact::RootActivity {
                scope: scope(),
                activity: *activity,
            })
        }
        PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootPhase { phase }) => {
            let scope = scope();
            matches!(
                phase,
                TurnPhase::Ready | TurnPhase::Interrupted | TurnPhase::Failed
            )
            .then(|| ConversationActivityFact::Terminal {
                scope: scope.clone(),
                phase: phase.clone(),
            })
            .or_else(|| {
                Some(ConversationActivityFact::RootPhase {
                    scope,
                    phase: phase.clone(),
                })
            })
        }
        PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity }) => {
            Some(ConversationActivityFact::WorkPhase {
                scope: scope(),
                work_id: activity.tool_use_id.clone(),
                kind: ActivityWorkKind::Command,
                phase: work_phase(&activity.phase),
            })
        }
        _ => None,
    }
}

const fn work_phase(phase: &ToolPhase) -> WorkPhase {
    match phase {
        ToolPhase::Started => WorkPhase::Running,
        ToolPhase::WaitingPermission => WorkPhase::WaitingPermission,
        ToolPhase::Completed => WorkPhase::Done,
        ToolPhase::Failed => WorkPhase::Failed,
    }
}

fn terminal(fact: &PublicWireFact) -> bool {
    matches!(
        fact,
        PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootPhase { phase })
            if matches!(phase, TurnPhase::Ready | TurnPhase::Interrupted | TurnPhase::Failed)
    )
}

fn invariant(message: &str) -> RuntimeError {
    RuntimeError::Ledger(gent_ports::LedgerError::Invariant(message.into()))
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
