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
        if let Some(reasoning) = self.reasoning.receive(frame) {
            facts = reasoning;
        }
        facts.retain(|fact| !matches!(fact, PublicWireFact::SessionStarted { .. }));
        if let Some(resets_at) = facts::rate_limit_resets_at(frame) {
            self.rate_limit_resets_at = Some(resets_at);
        }
        facts::attach_resets_at(&mut facts, self.rate_limit_resets_at);
        let child_thread = facts::child_thread(
            frame,
            &self.child_parent_by_thread,
            self.session.thread_id(),
        );
        let commands_drained = self
            .commands
            .observe(frame, child_thread.is_none(), &mut facts);
        if let Some(thread_id) = child_thread {
            let parent = self
                .child_parent_by_thread
                .get(thread_id)
                .map(String::as_str);
            facts = facts::attribute_to_child(facts, parent);
        }
        facts.retain(|fact| match fact {
            PublicWireFact::Event(NormalizedProviderEvent::ToolOutputDelta {
                tool_use_id, ..
            }) => {
                self.tool_output_item_ids.insert(tool_use_id.clone());
                true
            }
            PublicWireFact::Event(NormalizedProviderEvent::ChildStarted {
                child_id,
                parent_tool_use_id,
            }) => {
                let new = !self.child_parent_by_thread.contains_key(child_id);
                if new {
                    self.child_parent_by_thread
                        .insert(child_id.clone(), parent_tool_use_id.clone());
                }
                new
            }
            _ => true,
        });
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
        let mut child_settled = false;
        for (child_id, phase) in terminal
            .into_iter()
            .chain(facts::reported_child_terminals(frame))
        {
            if self.child_parent_by_thread.contains_key(&child_id)
                && self.settled_child_threads.insert(child_id.clone())
            {
                child_settled = true;
                effects.push(CodexTurnEffect::Fact(PublicWireFact::Event(
                    NormalizedProviderEvent::ChildTerminal { child_id, phase },
                )));
            }
        }
        if (commands_drained || (child_settled && self.commands.drained())) && !self.children_open()
        {
            effects.push(CodexTurnEffect::SessionState { idle: true });
        }
        effects
    }
}
