//! Conversation activity fact mapping for daemon-normalized Claude facts.

use gent_drivers::public_protocol::PublicWireFact;
use gent_types::{
    ActivityWorkKind, ConversationActivityFact, ConversationActivityScope, HostEpoch,
    NormalizedLifecycleSignal, NormalizedProviderEvent, TurnPhase,
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
        PublicWireFact::Event(NormalizedProviderEvent::ContextUsage { used_tokens, window_tokens }) => Some(ConversationActivityFact::ContextUsage {
            scope: scope(),
            used_tokens: *used_tokens,
            window_tokens: *window_tokens,
        }),
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
            Some(ConversationActivityFact::ToolActivity {
                scope: scope(),
                activity: activity.clone(),
            })
        }
        PublicWireFact::Event(NormalizedProviderEvent::ChildStarted { child_id, parent_tool_use_id }) => Some(ConversationActivityFact::SubagentStarted {
            scope: scope(), child_id: child_id.clone(), parent_tool_use_id: parent_tool_use_id.clone(),
        }),
        PublicWireFact::Event(NormalizedProviderEvent::ChildTerminal { child_id, phase })
        | PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ChildPhase { child_id, phase }) => Some(ConversationActivityFact::WorkPhase {
            scope: scope(), work_id: child_id.clone(), kind: ActivityWorkKind::Subagent, phase: phase.clone(),
        }),
        PublicWireFact::Event(NormalizedProviderEvent::CommandTerminal { command_id, phase })
        | PublicWireFact::Lifecycle(NormalizedLifecycleSignal::CommandPhase { command_id, phase }) => Some(ConversationActivityFact::WorkPhase {
            scope: scope(), work_id: command_id.clone(), kind: ActivityWorkKind::Command, phase: phase.clone(),
        }),
        PublicWireFact::Event(NormalizedProviderEvent::DecisionSettled { decision_id }) => Some(ConversationActivityFact::DecisionSettled {
            scope: scope(), decision_id: decision_id.clone(),
        }),
        _ => None,
    }
}
