use serde_json::{Value, json};

use super::phase::{CodexSessionPhase, matches_response};
use super::wire::{encode, response_id_at, thread_request};
use super::{CodexAppServerSession, CodexSessionError, CodexSessionIngress, MAX_PROMPT_BYTES};

const JSON_RPC_INVALID_REQUEST: i64 = -32600;

impl CodexAppServerSession {
    pub(super) fn response(
        &mut self,
        response_id: u64,
        frame: &Value,
    ) -> Result<CodexSessionIngress, CodexSessionError> {
        let phase = self.phase.clone();
        if !matches_response(&phase, response_id) {
            return Err(CodexSessionError::UncorrelatedResponse);
        }
        if let CodexSessionPhase::AwaitInjection {
            thread_id,
            turn_options,
            prompt,
            attachments,
            interrupted_reply,
            ..
        } = phase
        {
            self.phase = CodexSessionPhase::Ready {
                thread_id,
                turn_id: None,
                interrupt_request_id: None,
                turn_options,
            };
            let prompt = if frame.get("error").is_some() {
                crate::conversation_context_input::render_interrupted_reply_input(
                    &interrupted_reply,
                    &prompt,
                    MAX_PROMPT_BYTES,
                )
                .unwrap_or(prompt)
            } else {
                prompt
            };
            return Ok(CodexSessionIngress::Send(vec![
                self.start_turn_with_attachments(&prompt, &attachments)?,
            ]));
        }
        if let Some(confirmation) = self.confirm_resume_unavailable(&phase, frame)? {
            return Ok(confirmation);
        }
        if frame.get("error").is_some() {
            self.phase = CodexSessionPhase::Failed;
            return Err(CodexSessionError::RequestRejected);
        }
        if matches!(phase, CodexSessionPhase::AwaitCompaction { .. }) {
            let (next, ingress) = super::compaction::settled(phase, frame);
            self.phase = next;
            return Ok(ingress);
        }
        self.settle(phase, frame)
    }

    fn confirm_resume_unavailable(
        &mut self,
        phase: &CodexSessionPhase,
        frame: &Value,
    ) -> Result<Option<CodexSessionIngress>, CodexSessionError> {
        let invalid_request = frame
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_i64)
            == Some(JSON_RPC_INVALID_REQUEST);
        match phase {
            CodexSessionPhase::AwaitThread {
                resumed_thread_id: Some(thread_id),
                ..
            } if invalid_request => {
                let request_id = self.take_request_id()?;
                self.phase = CodexSessionPhase::ConfirmResumeUnavailable { request_id };
                Ok(Some(CodexSessionIngress::Send(vec![encode(&json!({
                    "id": request_id,
                    "method": "thread/turns/list",
                    "params": {"threadId": thread_id, "limit": 1}
                }))?])))
            }
            CodexSessionPhase::ConfirmResumeUnavailable { .. } => {
                self.phase = CodexSessionPhase::Failed;
                Err(if invalid_request {
                    CodexSessionError::ResumedThreadUnavailable
                } else {
                    CodexSessionError::RequestRejected
                })
            }
            _ => Ok(None),
        }
    }

    fn settle(
        &mut self,
        phase: CodexSessionPhase,
        frame: &Value,
    ) -> Result<CodexSessionIngress, CodexSessionError> {
        match phase {
            CodexSessionPhase::AwaitInitialize { config, .. } => {
                if frame.get("result").is_none() {
                    return Err(CodexSessionError::MalformedResponse);
                }
                let thread_request_id = self.take_request_id()?;
                let (method, params, resumed_thread_id, turn_options) = thread_request(config);
                self.phase = CodexSessionPhase::AwaitThread {
                    request_id: thread_request_id,
                    resumed_thread_id,
                    turn_options,
                };
                Ok(CodexSessionIngress::Send(vec![
                    encode(&json!({"method": "initialized", "params": {}}))?,
                    encode(&json!({"id": thread_request_id, "method": method, "params": params}))?,
                ]))
            }
            CodexSessionPhase::AwaitThread {
                resumed_thread_id,
                turn_options,
                ..
            } => {
                let thread_id = response_id_at(frame, "thread")?;
                if resumed_thread_id
                    .as_deref()
                    .is_some_and(|expected| expected != thread_id)
                {
                    self.phase = CodexSessionPhase::Failed;
                    return Err(CodexSessionError::ResumedThreadMismatch);
                }
                self.phase = CodexSessionPhase::Ready {
                    thread_id: thread_id.clone(),
                    turn_id: None,
                    interrupt_request_id: None,
                    turn_options,
                };
                Ok(CodexSessionIngress::Ready { thread_id })
            }
            CodexSessionPhase::AwaitTurn {
                thread_id,
                announced_turn_id,
                turn_options,
                ..
            } => {
                let turn_id = response_id_at(frame, "turn")?;
                if announced_turn_id
                    .as_deref()
                    .is_some_and(|announced| announced != turn_id)
                {
                    self.phase = CodexSessionPhase::Failed;
                    return Err(CodexSessionError::TurnIdentityMismatch);
                }
                self.phase = CodexSessionPhase::Ready {
                    thread_id,
                    turn_id: Some(turn_id),
                    interrupt_request_id: None,
                    turn_options,
                };
                Ok(CodexSessionIngress::TurnStarted)
            }
            CodexSessionPhase::Ready {
                thread_id,
                turn_id,
                interrupt_request_id: Some(_),
                turn_options,
            } => {
                self.phase = CodexSessionPhase::Ready {
                    thread_id,
                    turn_id,
                    interrupt_request_id: None,
                    turn_options,
                };
                Ok(CodexSessionIngress::Ignored)
            }
            _ => Err(CodexSessionError::UncorrelatedResponse),
        }
    }
}
