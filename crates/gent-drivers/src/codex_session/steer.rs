use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use super::{CodexSessionError, CodexSessionPhase, MAX_PROMPT_BYTES, wire::encode};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodexSteerOutcome {
    Consumed { message_id: String },
    Rejected { message_id: String },
    Unconfirmed { message_id: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CodexSteerResponse {
    Unrelated,
    Accepted,
    Outcome(CodexSteerOutcome),
}

fn steerable_turn(phase: &CodexSessionPhase) -> Option<(&String, &String)> {
    match phase {
        CodexSessionPhase::Ready {
            thread_id,
            turn_id: Some(turn_id),
            interrupt_request_id: None,
            ..
        } => Some((thread_id, turn_id)),
        _ => None,
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct CodexSteers {
    requests: BTreeMap<u64, String>,
    outstanding: Vec<OutstandingSteer>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OutstandingSteer {
    message: String,
    thread: String,
    turn: String,
}

impl CodexSteers {
    fn position(&self, message_id: &str) -> Option<usize> {
        self.outstanding
            .iter()
            .position(|steer| steer.message == message_id)
    }

    fn forget(&mut self, message_id: &str) -> Option<OutstandingSteer> {
        self.position(message_id)
            .map(|index| self.outstanding.remove(index))
    }

    pub(super) fn request(
        &mut self,
        phase: &CodexSessionPhase,
        request_id: u64,
        message_id: &str,
        prompt: &str,
        attachments: &[Value],
    ) -> Result<Vec<u8>, CodexSessionError> {
        let (thread_id, turn_id) = steerable_turn(phase).ok_or(CodexSessionError::TurnNotActive)?;
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
        let steer = OutstandingSteer {
            message: message_id.into(),
            thread: thread_id.clone(),
            turn: turn_id.clone(),
        };
        self.forget(message_id);
        self.outstanding.push(steer);
        Ok(frame)
    }

    pub(super) fn steerable(phase: &CodexSessionPhase) -> bool {
        steerable_turn(phase).is_some()
    }

    pub(super) fn response(
        &mut self,
        phase: &CodexSessionPhase,
        response_id: u64,
        frame: &Value,
    ) -> CodexSteerResponse {
        let Some(message_id) = self.requests.remove(&response_id) else {
            return CodexSteerResponse::Unrelated;
        };
        if self.position(&message_id).is_none() {
            return CodexSteerResponse::Accepted;
        }
        let turn_live = matches!(
            phase,
            CodexSessionPhase::Ready {
                turn_id: Some(_),
                ..
            }
        );
        if frame.get("error").is_none() && turn_live {
            return CodexSteerResponse::Accepted;
        }
        self.forget(&message_id);
        CodexSteerResponse::Outcome(if frame.get("error").is_some() {
            CodexSteerOutcome::Rejected { message_id }
        } else {
            CodexSteerOutcome::Unconfirmed { message_id }
        })
    }

    pub(super) fn consumed(&mut self, frame: &Value) -> Option<CodexSteerOutcome> {
        if !matches!(
            frame.get("method").and_then(Value::as_str),
            Some("item/started" | "item/completed")
        ) {
            return None;
        }
        let item = frame.pointer("/params/item")?;
        if item.get("type").and_then(Value::as_str) != Some("userMessage") {
            return None;
        }
        let message_id = item.get("clientId").and_then(Value::as_str)?;
        let steer = &self.outstanding[self.position(message_id)?];
        if frame.pointer("/params/threadId").and_then(Value::as_str) != Some(steer.thread.as_str())
            || frame.pointer("/params/turnId").and_then(Value::as_str) != Some(steer.turn.as_str())
        {
            return None;
        }
        self.forget(message_id)
            .map(|steer| CodexSteerOutcome::Consumed {
                message_id: steer.message,
            })
    }

    pub(super) fn turn_ended(&mut self) -> Vec<CodexSteerOutcome> {
        let awaiting_response = self.requests.values().cloned().collect::<BTreeSet<_>>();
        let (awaiting, unconsumed) = std::mem::take(&mut self.outstanding)
            .into_iter()
            .partition(|steer| awaiting_response.contains(&steer.message));
        self.outstanding = awaiting;
        unconsumed
            .into_iter()
            .map(|steer: OutstandingSteer| CodexSteerOutcome::Unconfirmed {
                message_id: steer.message,
            })
            .collect()
    }
}
