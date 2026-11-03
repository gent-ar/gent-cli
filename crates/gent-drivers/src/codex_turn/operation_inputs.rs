use std::collections::{BTreeMap, VecDeque};

use gent_types::{NormalizedLifecycleSignal, NormalizedProviderEvent, ToolPhase};
use serde_json::Value;

use super::CodexTurnEffect;
use crate::{codex_control::CodexControlRequest, public_protocol::PublicWireFact};

const MAX_ACTIVE_INPUTS: usize = 256;
const MAX_INPUT_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct OperationInputs {
    inputs: BTreeMap<(String, String, String), Value>,
    order: VecDeque<(String, String, String)>,
}

impl OperationInputs {
    pub(super) fn enrich(&self, frame: &Value, request: &mut CodexControlRequest) {
        if !request.method.ends_with("/requestApproval") {
            return;
        }
        let Some(key) = key(frame, &request.tool_use_id) else {
            return;
        };
        if let Some(cached) = self.inputs.get(&key) {
            request.input = merge(cached, request.input.as_ref());
        }
    }

    pub(super) fn observe(&mut self, frame: &Value, effects: &mut Vec<CodexTurnEffect>) {
        let mut limit_reached = false;
        for effect in effects.iter_mut() {
            let CodexTurnEffect::Fact(PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::ToolActivity { activity },
            )) = effect
            else {
                continue;
            };
            let Some(key) = key(frame, &activity.tool_use_id) else {
                continue;
            };
            if let Some(cached) = self.inputs.get(&key) {
                activity.input = merge(cached, activity.input.as_ref());
            }
            if matches!(activity.phase, ToolPhase::Completed | ToolPhase::Failed) {
                self.inputs.remove(&key);
                self.order.retain(|existing| existing != &key);
            } else if let Some(input) = &activity.input {
                if !self.inputs.contains_key(&key) {
                    if self.inputs.len() >= MAX_ACTIVE_INPUTS {
                        if let Some(oldest) = self.order.pop_front() {
                            self.inputs.remove(&oldest);
                        }
                        limit_reached = true;
                    }
                    self.order.push_back(key.clone());
                }
                self.inputs.insert(key, input.clone());
            }
        }
        if limit_reached {
            effects.push(CodexTurnEffect::Fact(PublicWireFact::Event(
                NormalizedProviderEvent::TransportDiagnostic {
                    classification: "codexOperationInputLimit".into(),
                },
            )));
        }
    }
}

fn key(frame: &Value, item_id: &str) -> Option<(String, String, String)> {
    let thread = frame.pointer("/params/threadId")?.as_str()?;
    let turn = frame.pointer("/params/turnId")?.as_str()?;
    if [thread, turn, item_id]
        .iter()
        .any(|value| value.is_empty() || value.len() > 1024)
    {
        return None;
    }
    Some((thread.into(), turn.into(), item_id.into()))
}

fn merge(cached: &Value, current: Option<&Value>) -> Option<Value> {
    let current = current
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let current_bytes = serde_json::to_vec(&current).ok()?.len();
    let budget = MAX_INPUT_BYTES
        .saturating_sub(current_bytes)
        .saturating_sub(16);
    let mut merged = gent_types::operation_preview_with_limit(cached, budget)
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    merged.extend(current);
    Some(Value::Object(merged))
}
