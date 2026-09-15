//! Bounded state and encoding for one Codex app-server connection.
//!
//! Provider-native identifiers stay private to this driver; its caller receives only
//! encoded frames and lifecycle facts. Process ownership, persistence, and provider launch live
//! outside this module.
use serde_json::{Value, json};

mod compaction;
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
/// Pure single-threaded Codex app-server connection state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexAppServerSession {
    phase: CodexSessionPhase,
    next_request_id: u64,
    steers: steer::CodexSteers,
}
impl CodexAppServerSession {
    /// Creates the state and its first required `initialize` request.
    ///
    /// # Errors
    /// Rejects unbounded or empty caller-provided connection fields.
    pub fn start(config: CodexSessionConfig) -> Result<(Self, Vec<u8>), CodexSessionError> {
        validate_config(&config)?;
        let request_id = 1;
        Ok((
            Self {
                phase: CodexSessionPhase::AwaitInitialize { request_id, config },
                next_request_id: 2,
                steers: steer::CodexSteers::default(),
            },
            encode(&codex_initialize_request(request_id))?,
        ))
    }

    /// Encodes a user turn only after an exact thread response established the native thread.
    ///
    /// # Errors
    /// Rejects an out-of-order turn, a concurrent turn, an empty prompt, or request-ID exhaustion.
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

    /// Reduces one parsed app-server frame, accepting responses only for the outstanding request.
    ///
    /// # Errors
    /// Rejects malformed, failed, or uncorrelated responses without retaining server payloads.
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
        if let Some(outcome) = self.steers.response(&self.phase, response_id, frame) {
            return Ok(outcome.map_or(CodexSessionIngress::Ignored, CodexSessionIngress::Steer));
        }
        self.response(response_id, frame)
    }
    /// Whether an exact thread response has made the connection available for a user turn.
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
    /// Validates a prompt before a daemon-owned process is launched.
    ///
    /// # Errors
    /// Rejects an empty or unbounded prompt without retaining its content.
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

    pub(crate) fn unconsumed_steers(&mut self) -> Vec<CodexSteerOutcome> {
        self.steers.turn_ended()
    }

    /// Requests documented cooperative interruption for exactly the live native turn.
    ///
    /// # Errors
    /// Rejects interruption before a turn is live or while an earlier request is unsettled.
    pub fn interrupt(&mut self) -> Result<Vec<u8>, CodexSessionError> {
        interrupt::request(&mut self.phase, &mut self.next_request_id)
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
