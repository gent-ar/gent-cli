//! Common durable recording of one already-normalized Claude or Codex wire fact.
//!
//! The current ledger exposes independent lifecycle, transcript, and activity ingresses. This
//! adapter preserves their durable order and returns every allocated cursor before a future
//! session supervisor may publish a delta. Stable IDs make an interrupted batch recoverable; a
//! true all-or-nothing cross-projection transaction remains a separate ledger-port requirement.

use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::Ledger;
use gent_runtime::{
    AgentChatTranscriptAppendRequest, AgentChatTranscriptAppendResult, ProviderActivityFact,
    RuntimeError,
};
use gent_types::{
    ActivityWorkKind, AgentChatConversationId, AgentChatRunId, ConversationActivityFact,
    ConversationActivityScope, HostEpoch, NormalizedLifecycleSignal, NormalizedProviderEvent,
    NormalizedTranscriptKind, ToolPhase, TurnPhase, WorkPhase,
};

use super::{PublicDriverFact, PublicDriverFactResult, PublicDriversRuntime};

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
        + gent_ports::RunProjectionLedger
        + gent_ports::ConversationActivityLedger
        + gent_ports::TranscriptLedger
        + gent_ports::AgentChatPromptDispatchLedger,
    D: gent_ports::PublicProviderRunner + Clone,
    R: gent_ports::PublicProviderResolver,
{
    /// Persists lifecycle, transcript, then activity records for a normalized provider fact.
    ///
    /// It intentionally does not settle a prompt: the lifecycle owner must observe a terminal
    /// signal and prove its process/session is drained before calling the existing settlement API.
    pub(crate) fn record_normalized_session(
        &self,
        coordinator_id: &str,
        input: &NormalizedSessionFact,
    ) -> Result<NormalizedSessionRecord, RuntimeError> {
        validate(input)?;
        let lifecycle = self.record(
            input.run_id.clone(),
            coordinator_id,
            input.host_epoch,
            PublicDriverFact::PublicWire {
                event_id: input.lifecycle_event_id.clone(),
                fact: input.fact.clone(),
            },
        )?;
        let lifecycle_cursor = self.cursor(&input.lifecycle_event_id)?;
        let transcript_cursor = output(&input.fact)
            .map(|(text, is_partial)| {
                let record = self.record(
                    input.run_id.clone(),
                    coordinator_id,
                    input.host_epoch,
                    PublicDriverFact::Transcript(AgentChatTranscriptAppendRequest {
                        conversation_id: AgentChatConversationId(input.conversation_id.clone()),
                        run_id: AgentChatRunId(input.run_id.clone()),
                        turn_id: input.turn_id.clone(),
                        event_id: input.transcript_event_id.clone(),
                        kind: NormalizedTranscriptKind::AssistantMessage,
                        text,
                        is_partial,
                    }),
                )?;
                match record {
                    PublicDriverFactResult::Transcript(
                        AgentChatTranscriptAppendResult::Persisted(event),
                    ) => Ok(event.cursor),
                    PublicDriverFactResult::Transcript(
                        AgentChatTranscriptAppendResult::DeniedObserver,
                    ) => Err(invariant(
                        "approved transcript ingress denied normalized session",
                    )),
                    _ => unreachable!("transcript facts use transcript ingress"),
                }
            })
            .transpose()?;
        let activity_cursor = activity(input)
            .map(|activity| {
                self.record(
                    input.run_id.clone(),
                    coordinator_id,
                    input.host_epoch,
                    PublicDriverFact::Activity(ProviderActivityFact {
                        event_id: input.activity_event_id.clone(),
                        activity,
                    }),
                )?;
                self.cursor(&input.activity_event_id)
            })
            .transpose()?;
        let PublicDriverFactResult::Lifecycle(_) = lifecycle else {
            unreachable!("public wire facts always use lifecycle ingress");
        };
        Ok(NormalizedSessionRecord {
            lifecycle_cursor,
            transcript_cursor,
            activity_cursor,
            terminal_signal: terminal(&input.fact),
        })
    }

    fn cursor(&self, event_id: &str) -> Result<u64, RuntimeError> {
        self.ledger
            .find_event(event_id)?
            .map(|event| event.cursor)
            .filter(|cursor| *cursor > 0)
            .ok_or_else(|| invariant("normalized session source was not persisted"))
    }
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
