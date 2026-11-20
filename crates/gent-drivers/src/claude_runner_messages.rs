use std::collections::BTreeMap;

use serde_json::Value;

use super::ClaudeRunnerEffect;

#[derive(Debug, Default)]
pub(super) struct WrittenMessages {
    written: BTreeMap<String, Written>,
    writes: BTreeMap<String, u32>,
}

#[derive(Debug)]
struct Written {
    message_id: String,
    started: bool,
    withdrawing: bool,
}

impl WrittenMessages {
    pub(super) fn identity(&self, message_id: &str) -> String {
        match self.writes.get(message_id) {
            None => message_id.to_owned(),
            Some(writes) => format!("{message_id}\0{writes}"),
        }
    }

    pub(super) fn written(&mut self, message_id: &str) {
        self.written.insert(
            super::input::message_uuid(&self.identity(message_id)),
            Written {
                message_id: message_id.into(),
                started: false,
                withdrawing: false,
            },
        );
        *self.writes.entry(message_id.into()).or_default() += 1;
    }

    pub(super) fn withdraw(&mut self, message_id: &str) -> Option<String> {
        let (uuid, written) = self.written.iter_mut().find(|(_, written)| {
            written.message_id == message_id && !written.started && !written.withdrawing
        })?;
        written.withdrawing = true;
        Some(uuid.clone())
    }

    pub(super) fn lifecycle(
        &mut self,
        uuid: Option<&Value>,
        state: &str,
    ) -> Vec<ClaudeRunnerEffect> {
        let Some(uuid) = uuid.and_then(Value::as_str) else {
            return Vec::new();
        };
        let Some(written) = self
            .written
            .get_mut(uuid)
            .filter(|written| !written.started)
        else {
            return Vec::new();
        };
        match state {
            "started" => {
                written.started = true;
                vec![ClaudeRunnerEffect::MessageStarted {
                    message_id: written.message_id.clone(),
                }]
            }
            "cancelled" => self
                .written
                .remove(uuid)
                .map(|written| ClaudeRunnerEffect::MessageCancelled {
                    message_id: written.message_id,
                })
                .into_iter()
                .collect(),
            _ => Vec::new(),
        }
    }

    pub(super) fn turn_started(&self, frame: &Value) -> Vec<ClaudeRunnerEffect> {
        let init = frame.get("type").and_then(Value::as_str) == Some("system")
            && frame.get("subtype").and_then(Value::as_str) == Some("init");
        if init && !self.written.values().any(|written| written.started) {
            return vec![ClaudeRunnerEffect::ContinuationStarted];
        }
        Vec::new()
    }

    pub(super) fn result(&mut self, frame: &Value) -> ClaudeRunnerEffect {
        let listed = frame
            .get("user_message_uuids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        let consumed = self
            .written
            .iter()
            .filter(|(uuid, written)| written.started || listed.contains(&uuid.as_str()))
            .map(|(uuid, _)| uuid.clone())
            .collect::<Vec<_>>();
        let message_ids = consumed
            .iter()
            .filter_map(|uuid| self.written.remove(uuid))
            .map(|written| written.message_id)
            .collect();
        ClaudeRunnerEffect::TurnResult {
            message_ids,
            continuation: frame.pointer("/origin/kind").and_then(Value::as_str)
                == Some("task-notification"),
        }
    }
}

#[cfg(test)]
#[path = "claude_runner_messages_tests.rs"]
mod tests;
