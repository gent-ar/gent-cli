//! Bounded state and encoding for one Codex app-server connection.
//!
//! This module follows Codex 0.144.1's `initialize -> thread/start|resume -> turn/start`
//! sequence. Provider-native identifiers stay private to this driver; its caller receives only
//! encoded frames and lifecycle facts. Process ownership, persistence, and provider launch live
//! outside this module.

use serde_json::{Value, json};

mod phase;
mod types;

pub use types::{CodexSessionConfig, CodexSessionError, CodexSessionIngress};

use phase::{CodexSessionPhase, matches_response};

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
                    "capabilities": {}
                }
            }))?,
        ))
    }

    /// Encodes a user turn only after an exact thread response established the native thread.
    ///
    /// # Errors
    /// Rejects an out-of-order turn, a concurrent turn, an empty prompt, or request-ID exhaustion.
    pub fn start_turn(&mut self, prompt: &str) -> Result<Vec<u8>, CodexSessionError> {
        Self::validate_prompt(prompt)?;
        let thread_id = match &self.phase {
            CodexSessionPhase::Ready {
                thread_id,
                turn_id: None,
            } => thread_id.clone(),
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
        };
        encode(&json!({
            "id": request_id,
            "method": "turn/start",
            "params": {"threadId": thread_id, "input": [{"type": "text", "text": prompt}]}
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
        matches!(self.phase, CodexSessionPhase::Ready { turn_id: None, .. })
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
                let (method, params, resumed_thread_id) = thread_request(config);
                self.phase = CodexSessionPhase::AwaitThread {
                    request_id: thread_request_id,
                    resumed_thread_id,
                };
                Ok(CodexSessionIngress::Send(vec![
                    encode(&json!({"method": "initialized", "params": {}}))?,
                    encode(&json!({"id": thread_request_id, "method": method, "params": params}))?,
                ]))
            }
            CodexSessionPhase::AwaitThread {
                resumed_thread_id, ..
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
                    thread_id,
                    turn_id: None,
                };
                Ok(CodexSessionIngress::Ready)
            }
            CodexSessionPhase::AwaitTurn {
                thread_id,
                announced_turn_id,
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
                };
                Ok(CodexSessionIngress::TurnStarted)
            }
            _ => Err(CodexSessionError::UncorrelatedResponse),
        }
    }

    fn notification(
        &mut self,
        method: &str,
        params: Option<&Value>,
    ) -> Result<CodexSessionIngress, CodexSessionError> {
        let Some(params) = params else {
            return Ok(CodexSessionIngress::Ignored);
        };
        let phase = self.phase.clone();
        match (method, phase) {
            (
                "turn/started",
                CodexSessionPhase::AwaitTurn {
                    request_id,
                    thread_id,
                    ..
                },
            ) if params.get("threadId").and_then(Value::as_str) == Some(thread_id.as_str()) => {
                let turn_id = nested_id(params, "turn")?;
                self.phase = CodexSessionPhase::AwaitTurn {
                    request_id,
                    thread_id,
                    announced_turn_id: Some(turn_id),
                };
                Ok(CodexSessionIngress::Ignored)
            }
            (
                "turn/completed",
                CodexSessionPhase::Ready {
                    thread_id,
                    turn_id: Some(turn_id),
                },
            ) if params.get("threadId").and_then(Value::as_str) == Some(thread_id.as_str())
                && nested_id(params, "turn")? == turn_id =>
            {
                self.phase = CodexSessionPhase::Ready {
                    thread_id,
                    turn_id: None,
                };
                Ok(CodexSessionIngress::TurnEnded)
            }
            _ => Ok(CodexSessionIngress::Ignored),
        }
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

fn validate_config(config: &CodexSessionConfig) -> Result<(), CodexSessionError> {
    optional_bounded(
        config.working_directory.as_deref(),
        MAX_WORKING_DIRECTORY_BYTES,
    )
    .then_some(())
    .ok_or(CodexSessionError::InvalidWorkingDirectory)?;
    optional_bounded(config.resume_thread_id.as_deref(), MAX_NATIVE_ID_BYTES)
        .then_some(())
        .ok_or(CodexSessionError::InvalidThreadId)
}

fn optional_bounded(value: Option<&str>, maximum: usize) -> bool {
    value.is_none_or(|value| !value.is_empty() && value.len() <= maximum)
}

fn thread_request(config: CodexSessionConfig) -> (&'static str, Value, Option<String>) {
    let mut params = config
        .working_directory
        .map_or_else(|| json!({}), |cwd| json!({"cwd": cwd}));
    match config.resume_thread_id {
        Some(thread_id) => {
            params["threadId"] = Value::String(thread_id.clone());
            ("thread/resume", params, Some(thread_id))
        }
        None => ("thread/start", params, None),
    }
}

fn response_id_at(frame: &Value, key: &str) -> Result<String, CodexSessionError> {
    frame
        .get("result")
        .and_then(|result| nested_id(result, key).ok())
        .ok_or(CodexSessionError::MalformedResponse)
}

fn nested_id(value: &Value, key: &str) -> Result<String, CodexSessionError> {
    let id = value
        .get(key)
        .and_then(Value::as_object)
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= MAX_NATIVE_ID_BYTES)
        .ok_or(CodexSessionError::MalformedResponse)?;
    Ok(id.to_owned())
}

fn encode(frame: &Value) -> Result<Vec<u8>, CodexSessionError> {
    let mut encoded = serde_json::to_vec(frame).map_err(|_| CodexSessionError::Serialization)?;
    encoded.push(b'\n');
    Ok(encoded)
}
