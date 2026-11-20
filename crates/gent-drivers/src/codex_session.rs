use serde_json::{Value, json};

mod compaction;
mod default_model;
mod interrupt;
mod notifications;
mod phase;
mod responses;
mod steer;
mod types;
mod wire;
pub use steer::CodexSteerOutcome;
pub use types::{
    CodexSandboxPolicy, CodexSessionConfig, CodexSessionError, CodexSessionIngress,
    CodexTurnEffort, CodexTurnOptions,
};

use crate::message_encoding::codex_initialize_request;
use phase::CodexSessionPhase;
use types::turn_parameters;
use wire::{encode, validate_config};
const MAX_NATIVE_ID_BYTES: usize = 512;
const MAX_WORKING_DIRECTORY_BYTES: usize = 4_096;
const MAX_PROMPT_BYTES: usize = 65_536;
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexAppServerSession {
    phase: CodexSessionPhase,
    next_request_id: u64,
    steers: steer::CodexSteers,
    terminal_cleans: std::collections::BTreeSet<u64>,
}
impl CodexAppServerSession {
    pub fn start(config: CodexSessionConfig) -> Result<(Self, Vec<u8>), CodexSessionError> {
        validate_config(&config)?;
        let request_id = 1;
        Ok((
            Self {
                phase: CodexSessionPhase::AwaitInitialize { request_id, config },
                next_request_id: 2,
                steers: steer::CodexSteers::default(),
                terminal_cleans: std::collections::BTreeSet::new(),
            },
            encode(&codex_initialize_request(request_id))?,
        ))
    }

    pub(crate) fn bind_external_sandbox(&mut self) -> Result<(), CodexSessionError> {
        let CodexSessionPhase::AwaitInitialize { config, .. } = &mut self.phase else {
            return Err(CodexSessionError::SessionFailed);
        };
        config.turn_options.bind_external_sandbox();
        Ok(())
    }

    pub fn start_turn(&mut self, prompt: &str) -> Result<Vec<u8>, CodexSessionError> {
        self.start_turn_with_attachments(prompt, &[])
    }

    pub fn start_turn_with_attachments(
        &mut self,
        prompt: &str,
        attachments: &[Value],
    ) -> Result<Vec<u8>, CodexSessionError> {
        Self::validate_prompt(prompt)?;
        let (thread_id, turn_options) = match &self.phase {
            CodexSessionPhase::Ready {
                thread_id,
                turn_id: None,
                interrupt_request_id: None,
                turn_options,
            } => (thread_id.clone(), turn_options.clone()),
            CodexSessionPhase::Ready { .. } | CodexSessionPhase::AwaitTurn { .. } => {
                return Err(CodexSessionError::TurnAlreadyActive);
            }
            CodexSessionPhase::Failed => return Err(CodexSessionError::SessionFailed),
            _ => return Err(CodexSessionError::ThreadNotReady),
        };
        let request_id = self.take_request_id()?;
        self.phase = CodexSessionPhase::AwaitTurn {
            request_id,
            thread_id: thread_id.clone(),
            announced_turn_id: None,
            turn_options: turn_options.clone(),
        };
        encode(&json!({
            "id": request_id,
            "method": "turn/start",
            "params": turn_parameters(&turn_options, &thread_id, prompt, attachments)
        }))
    }
    pub fn start_turn_after_interrupted_reply(
        &mut self,
        prompt: &str,
        attachments: &[Value],
        interrupted_reply: Option<&str>,
    ) -> Result<Vec<u8>, CodexSessionError> {
        if compaction::is_compaction(prompt, attachments) {
            return self.start_compaction();
        }
        let Some(interrupted_reply) = interrupted_reply.filter(|reply| !reply.trim().is_empty())
        else {
            return self.start_turn_with_attachments(prompt, attachments);
        };
        Self::validate_prompt(prompt)?;
        let (thread_id, turn_options) = match &self.phase {
            CodexSessionPhase::Ready {
                thread_id,
                turn_id: None,
                interrupt_request_id: None,
                turn_options,
            } => (thread_id.clone(), turn_options.clone()),
            CodexSessionPhase::Ready { .. }
            | CodexSessionPhase::AwaitTurn { .. }
            | CodexSessionPhase::AwaitInjection { .. } => {
                return Err(CodexSessionError::TurnAlreadyActive);
            }
            CodexSessionPhase::Failed => return Err(CodexSessionError::SessionFailed),
            _ => return Err(CodexSessionError::ThreadNotReady),
        };
        let request_id = self.take_request_id()?;
        let frame = encode(&json!({
            "id": request_id,
            "method": "thread/inject_items",
            "params": {
                "threadId": thread_id,
                "items": [{
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": interrupted_reply}]
                }]
            }
        }))?;
        self.phase = CodexSessionPhase::AwaitInjection {
            request_id,
            thread_id,
            turn_options,
            prompt: prompt.to_owned(),
            attachments: attachments.to_vec(),
            interrupted_reply: interrupted_reply.to_owned(),
        };
        Ok(frame)
    }

    pub fn receive(&mut self, frame: &Value) -> Result<CodexSessionIngress, CodexSessionError> {
        if let Some(method) = frame.get("method").and_then(Value::as_str) {
            if let Some(outcome) = self.steers.consumed(frame) {
                return Ok(CodexSessionIngress::Steer(outcome));
            }
            return self.notification(method, frame.get("params"));
        }
        let response_id = frame
            .get("id")
            .and_then(Value::as_u64)
            .filter(|id| *id > 0)
            .ok_or(CodexSessionError::MalformedResponse)?;
        if self.terminal_cleans.remove(&response_id) {
            return Ok(CodexSessionIngress::Ignored);
        }
        match self.steers.response(&self.phase, response_id, frame) {
            steer::CodexSteerResponse::Unrelated => self.response(response_id, frame),
            steer::CodexSteerResponse::Accepted => Ok(CodexSessionIngress::Ignored),
            steer::CodexSteerResponse::Outcome(outcome) => Ok(CodexSessionIngress::Steer(outcome)),
        }
    }
    #[must_use]
    pub fn is_ready(&self) -> bool {
        matches!(
            self.phase,
            CodexSessionPhase::Ready {
                turn_id: None,
                interrupt_request_id: None,
                ..
            }
        )
    }

    pub(crate) fn is_handshaking(&self) -> bool {
        matches!(
            self.phase,
            CodexSessionPhase::AwaitInitialize { .. }
                | CodexSessionPhase::AwaitThread { .. }
                | CodexSessionPhase::AwaitDefaultModel { .. }
                | CodexSessionPhase::ConfirmResumeUnavailable { .. }
        )
    }

    pub(crate) fn thread_id(&self) -> Option<&str> {
        match &self.phase {
            CodexSessionPhase::Ready { thread_id, .. }
            | CodexSessionPhase::AwaitTurn { thread_id, .. }
            | CodexSessionPhase::AwaitCompaction { thread_id, .. }
            | CodexSessionPhase::AwaitInjection { thread_id, .. } => Some(thread_id),
            _ => None,
        }
    }

    pub(crate) fn active_turn_id(&self) -> Option<&str> {
        match &self.phase {
            CodexSessionPhase::Ready {
                turn_id: Some(turn_id),
                ..
            }
            | CodexSessionPhase::AwaitTurn {
                announced_turn_id: Some(turn_id),
                ..
            }
            | CodexSessionPhase::AwaitCompaction {
                announced_turn_id: Some(turn_id),
                ..
            } => Some(turn_id),
            _ => None,
        }
    }
    pub fn validate_prompt(prompt: &str) -> Result<(), CodexSessionError> {
        (!prompt.is_empty() && prompt.len() <= MAX_PROMPT_BYTES)
            .then_some(())
            .ok_or(CodexSessionError::InvalidPrompt)
    }

    pub fn steer(
        &mut self,
        message_id: &str,
        prompt: &str,
        attachments: &[Value],
    ) -> Result<Vec<u8>, CodexSessionError> {
        let request_id = self.take_request_id()?;
        self.steers
            .request(&self.phase, request_id, message_id, prompt, attachments)
    }

    #[must_use]
    pub fn steerable(&self) -> bool {
        steer::CodexSteers::steerable(&self.phase)
    }

    pub(crate) fn unconsumed_steers(&mut self) -> Vec<CodexSteerOutcome> {
        self.steers.turn_ended()
    }

    pub fn interrupt(&mut self) -> Result<Vec<u8>, CodexSessionError> {
        interrupt::request(
            &mut self.phase,
            &mut self.next_request_id,
            &mut self.terminal_cleans,
        )
    }

    fn notification(
        &mut self,
        method: &str,
        params: Option<&Value>,
    ) -> Result<CodexSessionIngress, CodexSessionError> {
        notifications::reduce(&mut self.phase, method, params)
    }

    fn take_request_id(&mut self) -> Result<u64, CodexSessionError> {
        let request_id = self.next_request_id;
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .ok_or(CodexSessionError::RequestIdExhausted)?;
        Ok(request_id)
    }
}
