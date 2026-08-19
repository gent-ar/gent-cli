//! Conversation activity fact mapping for daemon-normalized Codex facts.

use gent_drivers::public_protocol::PublicWireFact;
use gent_types::{
    ActivityWorkKind, ConversationActivityFact, ConversationActivityScope, HostEpoch,
    NormalizedLifecycleSignal, NormalizedProviderEvent, ToolPhase, TurnPhase, WorkPhase,
};

use super::Binding;

pub(super) fn fact(
    binding: &Binding,
    host_epoch: HostEpoch,
    fact: &PublicWireFact,
) -> Option<ConversationActivityFact> {
    let scope = || ConversationActivityScope {
        conversation_id: binding.prompt.message.conversation_id.clone(),
        run_id: binding.prompt.run_id.0.clone(),
        turn_id: binding.prompt.message.turn_id.clone(),
        host_epoch,
        cursor: 0,
    };
    match fact {
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
            if matches!(
                phase,
                TurnPhase::Ready | TurnPhase::Interrupted | TurnPhase::Failed
            ) {
                Some(ConversationActivityFact::Terminal {
                    scope,
                    phase: phase.clone(),
                })
            } else {
                Some(ConversationActivityFact::RootPhase {
                    scope,
                    phase: phase.clone(),
                })
            }
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

fn work_phase(phase: &ToolPhase) -> WorkPhase {
    match phase {
        ToolPhase::Started => WorkPhase::Running,
        ToolPhase::WaitingPermission => WorkPhase::WaitingPermission,
        ToolPhase::Completed => WorkPhase::Done,
        ToolPhase::Failed => WorkPhase::Failed,
    }
}
