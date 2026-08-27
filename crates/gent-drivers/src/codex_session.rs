//! Bounded state and encoding for one Codex app-server connection.
//!
//! Provider-native identifiers stay private to this driver; its caller receives only
//! encoded frames and lifecycle facts. Process ownership, persistence, and provider launch live
//! outside this module.
use serde_json::{Value, json};

mod interrupt;
mod notifications;
mod phase;
mod types;
mod wire;
pub use types::{
    CodexSandboxPolicy, CodexSessionConfig, CodexSessionError, CodexSessionIngress,
    CodexTurnEffort, CodexTurnOptions,
};

use phase::{CodexSessionPhase, matches_response};
use types::turn_parameters;
use wire::{encode, response_id_at, thread_request, validate_config};
const MAX_NATIVE_ID_BYTES: usize = 512;
const MAX_WORKING_DIRECTORY_BYTES: usize = 4_096;
const MAX_PROMPT_BYTES: usize = 65_536;
/// Pure single-threaded Codex app-server connection state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexAppServerSession {
    phase: CodexSessionPhase,
    next_request_id: u64,
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
            },
            encode(&json!({
                "id": request_id,
                "method": "initialize",
                "params": {
                    "clientInfo": {"name": "gent", "version": env!("CARGO_PKG_VERSION")},
                    "capabilities": {"experimentalApi": true, "requestAttestation": false}
                }
            }))?,
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
    /// Reduces one parsed app-server frame, accepting responses only for the outstanding request.
    ///
    /// # Errors
    /// Rejects malformed, failed, or uncorrelated responses without retaining server payloads.
    pub fn receive(&mut self, frame: &Value) -> Result<CodexSessionIngress, CodexSessionError> {
        if let Some(method) = frame.get("method").and_then(Value::as_str) {
            return self.notification(method, frame.get("params"));
        }
        let response_id = frame
            .get("id")
            .and_then(Value::as_u64)
            .filter(|id| *id > 0)
            .ok_or(CodexSessionError::MalformedResponse)?;
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

    /// Requests documented cooperative interruption for exactly the live native turn.
    ///
    /// # Errors
    /// Rejects interruption before a turn is live or while an earlier request is unsettled.
    pub fn interrupt(&mut self) -> Result<Vec<u8>, CodexSessionError> {
        interrupt::request(&mut self.phase, &mut self.next_request_id)
    }

    fn response(
        &mut self,
        response_id: u64,
        frame: &Value,
    ) -> Result<CodexSessionIngress, CodexSessionError> {
        let phase = self.phase.clone();
        if !matches_response(&phase, response_id) {
            return Err(CodexSessionError::UncorrelatedResponse);
        }
        if frame.get("error").is_some() {
            self.phase = CodexSessionPhase::Failed;
            return Err(CodexSessionError::RequestRejected);
        }
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
