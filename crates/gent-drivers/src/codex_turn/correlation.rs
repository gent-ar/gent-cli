use gent_types::{NormalizedLifecycleSignal, NormalizedProviderEvent, RootActivity, TurnPhase};
use serde_json::Value;

use super::{CodexTurnDriver, CodexTurnEffect, facts};
use crate::PublicProvider;
use crate::public_protocol::{PublicWireFact, normalize_public_frame};

impl CodexTurnDriver {
    pub(super) fn facts(&mut self, frame: &Value) -> Vec<CodexTurnEffect> {
        let terminal = facts::child_terminal(frame, &self.child_parent_by_thread);
        let child_phase = facts::child_phase(frame, &self.child_parent_by_thread);
        let mut facts = if facts::is_empty_turn_completion(frame) {
            self.session.active_turn_id().map_or_else(
                || normalize_public_frame(PublicProvider::Codex, frame),
                |turn_id| {
                    vec![
                        PublicWireFact::Event(NormalizedProviderEvent::TurnEnded {
                            turn_id: turn_id.into(),
                        }),
                        PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootActivity {
                            activity: RootActivity::Idle,
                        }),
                        PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootPhase {
                            phase: TurnPhase::Ready,
                        }),
                    ]
                },
            )
        } else {
            normalize_public_frame(PublicProvider::Codex, frame)
        };
        if facts.iter().any(|fact| {
            matches!(
                fact,
                PublicWireFact::Event(NormalizedProviderEvent::TurnStarted { .. })
            )
        }) {
            self.reported_root_failure = false;
        }
        facts.retain(|fact| {
            if !matches!(
                fact,
                PublicWireFact::Event(NormalizedProviderEvent::ProviderFailure { .. })
            ) {
                return true;
            }
            if self.reported_root_failure {
                return false;
            }
            self.reported_root_failure = true;
            true
        });
        facts.retain(|fact| !matches!(fact, PublicWireFact::SessionStarted { .. }));
        facts::retain_root_context(frame, &self.child_parent_by_thread, &mut facts);
        if terminal.is_some() && matches!(facts::method(frame), Some("turn/completed")) {
            facts.retain(|fact| !facts::root_terminal_fact(fact));
        }
        for fact in &facts {
            if let PublicWireFact::Event(NormalizedProviderEvent::ToolOutputDelta {
                tool_use_id,
                ..
            }) = fact
            {
                self.tool_output_item_ids.insert(tool_use_id.clone());
            }
            if let PublicWireFact::Event(NormalizedProviderEvent::ChildStarted {
                child_id,
                parent_tool_use_id,
            }) = fact
            {
                self.child_parent_by_thread
                    .entry(child_id.clone())
                    .or_insert_with(|| parent_tool_use_id.clone());
            }
        }
        if let Some(fallback) =
            facts::command_completion_fallback(&mut self.tool_output_item_ids, frame)
        {
            facts.push(fallback);
        }
        if let Some((child_id, phase)) = child_phase
            && !matches!(
                phase,
                gent_types::WorkPhase::Done
                    | gent_types::WorkPhase::Failed
                    | gent_types::WorkPhase::Interrupted
            )
        {
            facts.push(PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::ChildPhase { child_id, phase },
            ));
        }
        let mut effects: Vec<_> = facts.into_iter().map(CodexTurnEffect::Fact).collect();
        if let Some((child_id, phase)) = terminal
            && self.settled_child_threads.insert(child_id.clone())
        {
            effects.push(CodexTurnEffect::Fact(PublicWireFact::Event(
                NormalizedProviderEvent::ChildTerminal { child_id, phase },
            )));
        }
        effects
    }
}
