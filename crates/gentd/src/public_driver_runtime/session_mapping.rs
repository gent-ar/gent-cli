use gent_drivers::public_protocol::PublicWireFact;
use gent_types::{
    ActivityWorkKind, ConversationActivityFact, ConversationActivityScope, HostEpoch,
    NormalizedLifecycleSignal, NormalizedProviderEvent, NormalizedSessionLifecycle,
    NormalizedTranscriptKind, TurnPhase,
};

use super::NormalizedSessionFact;

pub(super) fn transcript_content(
    fact: &PublicWireFact,
) -> Option<(NormalizedTranscriptKind, String, bool)> {
    match fact {
        PublicWireFact::Event(NormalizedProviderEvent::Output { text, is_partial }) => Some((
            NormalizedTranscriptKind::AssistantMessage,
            text.clone(),
            *is_partial,
        )),
        PublicWireFact::Event(NormalizedProviderEvent::Thinking { text, is_partial }) => Some((
            NormalizedTranscriptKind::Thinking,
            text.clone(),
            *is_partial,
        )),
        PublicWireFact::Event(NormalizedProviderEvent::ToolOutputDelta {
            text,
            is_partial,
            ..
        }) => Some((
            NormalizedTranscriptKind::ToolActivity,
            text.clone(),
            *is_partial,
        )),
        PublicWireFact::Event(NormalizedProviderEvent::ProviderFailure { message, .. }) => {
            Some((NormalizedTranscriptKind::Notice, message.clone(), false))
        }
        PublicWireFact::Event(NormalizedProviderEvent::PlanProposed { text }) => {
            Some((NormalizedTranscriptKind::Plan, text.clone(), false))
        }
        PublicWireFact::Event(NormalizedProviderEvent::TransportDiagnostic { classification }) => {
            diagnostic_notice(classification)
                .map(|notice| (NormalizedTranscriptKind::Notice, notice.into(), false))
        }
        _ => None,
    }
}

fn diagnostic_notice(classification: &str) -> Option<&'static str> {
    match classification {
        gent_types::OVERSIZED_PROVIDER_FRAME_DIAGNOSTIC => {
            Some(gent_types::OVERSIZED_PROVIDER_FRAME_NOTICE)
        }
        gent_types::PROVIDER_SESSION_RECOVERED_DIAGNOSTIC => {
            Some(gent_types::PROVIDER_SESSION_RECOVERED_NOTICE)
        }
        gent_types::PROVIDER_CONTEXT_COMPACTED_DIAGNOSTIC => {
            Some(gent_types::PROVIDER_CONTEXT_COMPACTED_NOTICE)
        }
        gent_types::PROVIDER_CONTEXT_COMPACTION_FAILED_DIAGNOSTIC => {
            Some(gent_types::PROVIDER_CONTEXT_COMPACTION_FAILED_NOTICE)
        }
        _ => None,
    }
}

pub(super) fn activity(input: &NormalizedSessionFact) -> Option<ConversationActivityFact> {
    activity_for_fact(
        &input.conversation_id,
        &input.run_id,
        &input.turn_id,
        input.host_epoch,
        &input.fact,
    )
}

pub(crate) fn activity_for_lifecycle(
    conversation_id: &str,
    run_id: &str,
    turn_id: &str,
    host_epoch: HostEpoch,
    lifecycle: &NormalizedSessionLifecycle,
) -> Option<ConversationActivityFact> {
    match lifecycle {
        NormalizedSessionLifecycle::Event { event } => activity_for_fact(
            conversation_id,
            run_id,
            turn_id,
            host_epoch,
            &PublicWireFact::Event(event.clone()),
        ),
        NormalizedSessionLifecycle::Signal { signal } => activity_for_fact(
            conversation_id,
            run_id,
            turn_id,
            host_epoch,
            &PublicWireFact::Lifecycle(signal.clone()),
        ),
    }
}

fn activity_for_fact(
    conversation_id: &str,
    run_id: &str,
    turn_id: &str,
    host_epoch: HostEpoch,
    fact: &PublicWireFact,
) -> Option<ConversationActivityFact> {
    let scope = || ConversationActivityScope {
        conversation_id: conversation_id.into(),
        run_id: run_id.into(),
        turn_id: turn_id.into(),
        host_epoch,
        cursor: 0,
    };
    match fact {
        PublicWireFact::Event(NormalizedProviderEvent::TurnStarted { .. }) => {
            Some(ConversationActivityFact::TurnStarted { scope: scope() })
        }
        PublicWireFact::Event(NormalizedProviderEvent::ContextUsage {
            used_tokens,
            window_tokens,
        }) => Some(ConversationActivityFact::ContextUsage {
            scope: scope(),
            used_tokens: *used_tokens,
            window_tokens: *window_tokens,
        }),
        PublicWireFact::Event(NormalizedProviderEvent::TokenUsage { usage }) => {
            Some(ConversationActivityFact::TokenUsage {
                scope: scope(),
                usage: *usage,
            })
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
                cause: None,
            })
            .or_else(|| {
                Some(ConversationActivityFact::RootPhase {
                    scope,
                    phase: phase.clone(),
                })
            })
        }
        PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity }) => {
            Some(ConversationActivityFact::ToolActivity {
                scope: scope(),
                activity: activity.clone(),
            })
        }
        PublicWireFact::Event(NormalizedProviderEvent::ChildStarted {
            child_id,
            parent_tool_use_id,
        }) => Some(ConversationActivityFact::SubagentStarted {
            scope: scope(),
            child_id: child_id.clone(),
            parent_tool_use_id: parent_tool_use_id.clone(),
        }),
        PublicWireFact::Event(NormalizedProviderEvent::ChildTerminal { child_id, phase })
        | PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ChildPhase { child_id, phase }) => {
            Some(ConversationActivityFact::WorkPhase {
                scope: scope(),
                work_id: child_id.clone(),
                kind: ActivityWorkKind::Subagent,
                phase: phase.clone(),
            })
        }
        PublicWireFact::Event(NormalizedProviderEvent::CommandTerminal { command_id, phase })
        | PublicWireFact::Lifecycle(NormalizedLifecycleSignal::CommandPhase {
            command_id,
            phase,
        }) => Some(ConversationActivityFact::WorkPhase {
            scope: scope(),
            work_id: command_id.clone(),
            kind: ActivityWorkKind::Command,
            phase: phase.clone(),
        }),
        PublicWireFact::Event(NormalizedProviderEvent::DecisionSettled { decision_id }) => {
            Some(ConversationActivityFact::DecisionSettled {
                scope: scope(),
                decision_id: decision_id.clone(),
            })
        }
        _ => None,
    }
}

pub(super) fn terminal(fact: &PublicWireFact) -> bool {
    matches!(
        fact,
        PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootPhase { phase })
            if matches!(phase, TurnPhase::Ready | TurnPhase::Interrupted | TurnPhase::Failed)
    )
}
