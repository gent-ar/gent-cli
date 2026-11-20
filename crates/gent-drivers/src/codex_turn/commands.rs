use std::collections::{BTreeMap, BTreeSet};

use gent_types::{NormalizedLifecycleSignal, ToolActivity, ToolPhase};
use serde_json::Value;

use crate::public_protocol::PublicWireFact;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct RunningCommands {
    running: BTreeMap<String, ToolActivity>,
    awaiting_approval: BTreeSet<String>,
    stopped: bool,
    thread_idle: bool,
}

impl RunningCommands {
    pub(super) fn stop(&mut self) {
        self.stopped = !self.running.is_empty();
    }

    pub(super) fn approval_requested(&mut self, tool_use_id: &str) {
        if self.running.contains_key(tool_use_id) {
            self.awaiting_approval.insert(tool_use_id.to_owned());
        }
    }

    pub(super) fn approval_answered(&mut self, tool_use_id: &str) {
        self.awaiting_approval.remove(tool_use_id);
    }

    pub(super) fn session_idle(&mut self, thread_idle: bool) -> bool {
        self.thread_idle = thread_idle;
        self.drained()
    }

    pub(super) fn drained(&self) -> bool {
        self.thread_idle && self.running.is_empty()
    }

    pub(super) fn observe(
        &mut self,
        frame: &Value,
        root: bool,
        facts: &mut Vec<PublicWireFact>,
    ) -> bool {
        if root && super::facts::method(frame) == Some("turn/completed") {
            let unapproved = if self.stopped {
                ToolPhase::Interrupted
            } else {
                ToolPhase::Failed
            };
            let mut settled: Vec<_> = std::mem::take(&mut self.awaiting_approval)
                .into_iter()
                .filter_map(|tool_use_id| self.running.remove(&tool_use_id))
                .map(|activity| phase(activity, unapproved.clone()))
                .collect();
            settled.extend(
                self.running
                    .values()
                    .cloned()
                    .map(|activity| phase(activity, ToolPhase::Background)),
            );
            facts.splice(0..0, settled);
            self.stopped &= !self.running.is_empty();
            return false;
        }
        if frame.pointer("/params/item/type").and_then(Value::as_str) != Some("commandExecution") {
            return false;
        }
        let mut ended = false;
        for fact in facts.iter_mut() {
            let PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity }) =
                fact
            else {
                continue;
            };
            if activity.phase == ToolPhase::Started {
                self.running
                    .insert(activity.tool_use_id.clone(), activity.clone());
                continue;
            }
            if self.running.remove(&activity.tool_use_id).is_some() {
                self.awaiting_approval.remove(&activity.tool_use_id);
                ended = true;
                if self.stopped && activity.phase == ToolPhase::Failed {
                    activity.phase = ToolPhase::Interrupted;
                }
            }
        }
        self.stopped &= !self.running.is_empty();
        ended && self.drained()
    }
}

fn phase(activity: ToolActivity, phase: ToolPhase) -> PublicWireFact {
    PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity {
        activity: ToolActivity { phase, ..activity },
    })
}
