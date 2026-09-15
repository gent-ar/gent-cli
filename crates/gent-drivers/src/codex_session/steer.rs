use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use super::{CodexSessionError, CodexSessionPhase, MAX_PROMPT_BYTES, wire::encode};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodexSteerOutcome {
    Consumed { message_id: String },
    Rejected { message_id: String },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct CodexSteers {
    requests: BTreeMap<u64, String>,
    outstanding: BTreeSet<String>,
}

impl CodexSteers {
    pub(super) fn request(
        &mut self,
        phase: &CodexSessionPhase,
        request_id: u64,
        message_id: &str,
        prompt: &str,
        attachments: &[Value],
    ) -> Result<Vec<u8>, CodexSessionError> {
        let CodexSessionPhase::Ready {
            thread_id,
            turn_id: Some(turn_id),
            interrupt_request_id: None,
            ..
        } = phase
        else {
            return Err(CodexSessionError::TurnNotActive);
        };
        if prompt.is_empty() || prompt.len() > MAX_PROMPT_BYTES || message_id.is_empty() {
            return Err(CodexSessionError::InvalidPrompt);
        }
        let input = std::iter::once(json!({"type": "text", "text": prompt}))
            .chain(attachments.iter().cloned())
            .collect::<Vec<_>>();
        let frame = encode(&json!({
            "id": request_id,
            "method": "turn/steer",
            "params": {
                "threadId": thread_id,
                "expectedTurnId": turn_id,
                "input": input,
                "clientUserMessageId": message_id,
            }
        }))?;
        self.requests.insert(request_id, message_id.into());
        self.outstanding.insert(message_id.into());
        Ok(frame)
    }

    pub(super) fn response(
        &mut self,
        phase: &CodexSessionPhase,
        response_id: u64,
        frame: &Value,
    ) -> Option<Option<CodexSteerOutcome>> {
        let message_id = self.requests.remove(&response_id)?;
        let turn_live = matches!(
            phase,
            CodexSessionPhase::Ready {
                turn_id: Some(_),
                ..
            }
        );
        if frame.get("error").is_none() && turn_live {
            return Some(None);
        }
        self.outstanding.remove(&message_id);
        Some(Some(CodexSteerOutcome::Rejected { message_id }))
    }

    pub(super) fn consumed(&mut self, frame: &Value) -> Option<CodexSteerOutcome> {
        if frame.get("method").and_then(Value::as_str) != Some("item/started") {
            return None;
        }
        let item = frame.pointer("/params/item")?;
        if item.get("type").and_then(Value::as_str) != Some("userMessage") {
            return None;
        }
        let message_id = item.get("clientId").and_then(Value::as_str)?;
        self.outstanding
            .remove(message_id)
            .then(|| CodexSteerOutcome::Consumed {
                message_id: message_id.into(),
            })
    }

    pub(super) fn turn_ended(&mut self) -> Vec<CodexSteerOutcome> {
        let awaiting_response = self.requests.values().cloned().collect::<BTreeSet<_>>();
        let unconsumed = self
            .outstanding
            .iter()
            .filter(|message_id| !awaiting_response.contains(*message_id))
            .cloned()
            .collect::<Vec<_>>();
        unconsumed
            .into_iter()
            .map(|message_id| {
                self.outstanding.remove(&message_id);
                CodexSteerOutcome::Rejected { message_id }
            })
            .collect()
    }
}
