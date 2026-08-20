//! Bounded JSON-RPC stdio framing for an upstream `claurst acp` process.
//!
//! This is deliberately below `PrivateClaurstBridge`: it proves the upstream handshake and
//! emits only safely-normalized output/thinking/attention facts.  The next composition layer
//! must own the ACP streams and correlate this transport's session/turn state to bridge cursors.

use std::{collections::VecDeque, path::Path};

use gent_types::{NormalizedLifecycleSignal, NormalizedProviderEvent};
use serde_json::{Value, json};

const MAX_ACP_FRAME_BYTES: usize = 256 * 1024;
const MAX_HANDSHAKE_FRAMES: usize = 32;

/// Nonblocking line-oriented stdio owned by the future ACP process adapter.
pub(crate) trait ClaurstAcpStdio {
    /// Writes one complete newline-delimited JSON-RPC frame.
    fn write_frame(&mut self, frame: &[u8]) -> Result<(), String>;
    /// Returns one already-delimited line, or `None` when no complete frame is presently queued.
    fn try_read_frame(&mut self, maximum_bytes: usize) -> Result<Option<Vec<u8>>, String>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClaurstAcpTerminal {
    Completed,
    Interrupted,
    Failed,
}

/// Bounded result from a single nonblocking ACP drain.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ClaurstAcpDrain {
    pub(crate) facts: Vec<ClaurstAcpFact>,
    pub(crate) terminal: Option<ClaurstAcpTerminal>,
}

/// Content-safe projection of only the upstream events Gent can presently normalize.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ClaurstAcpFact {
    Event(NormalizedProviderEvent),
    Lifecycle(NormalizedLifecycleSignal),
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub(crate) enum ClaurstAcpTransportError {
    #[error("Claurst ACP transport is not initialized")]
    Uninitialized,
    #[error("Claurst ACP returned an invalid JSON-RPC frame")]
    InvalidFrame,
    #[error("Claurst ACP frame exceeds the fixed bound")]
    FrameTooLarge,
    #[error("Claurst ACP handshake did not return its required response")]
    HandshakeIncomplete,
    #[error("Claurst ACP returned an invalid session identifier")]
    InvalidSession,
    #[error("Claurst ACP I/O failed: {0}")]
    Io(String),
}

/// One initialized ACP connection. It never accepts provider configuration from a caller.
pub(crate) struct ClaurstAcpTransport<S> {
    stdio: S,
    next_request_id: u64,
    initialized: bool,
    queued: VecDeque<ClaurstAcpFact>,
    pending_prompt_id: Option<u64>,
}

impl<S: ClaurstAcpStdio> ClaurstAcpTransport<S> {
    #[must_use]
    pub(crate) fn new(stdio: S) -> Self {
        Self {
            stdio,
            next_request_id: 1,
            initialized: false,
            queued: VecDeque::new(),
            pending_prompt_id: None,
        }
    }

    /// Completes upstream `initialize`, then opens one session rooted at Gent's absolute workspace.
    pub(crate) fn initialize_session(
        &mut self,
        workspace: &Path,
    ) -> Result<String, ClaurstAcpTransportError> {
        if !workspace.is_absolute() {
            return Err(ClaurstAcpTransportError::InvalidSession);
        }
        if !self.initialized {
            let initialize_id = self.send_request(
                "initialize",
                json!({
                    "protocolVersion": 1,
                    "clientCapabilities": {},
                    "clientInfo": {"name": "gent", "version": env!("CARGO_PKG_VERSION")},
                }),
            )?;
            self.wait_for_response(initialize_id)?;
            self.initialized = true;
        }
        let session_id =
            self.send_request("session/new", json!({"cwd": workspace, "mcpServers": []}))?;
        let response = self.wait_for_response(session_id)?;
        response
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && value.len() <= 256)
            .map(ToOwned::to_owned)
            .ok_or(ClaurstAcpTransportError::InvalidSession)
    }

    /// Dispatches `session/prompt` without waiting for its terminal response.
    pub(crate) fn prompt(
        &mut self,
        session_id: &str,
        prompt: &str,
    ) -> Result<(), ClaurstAcpTransportError> {
        if !self.initialized {
            return Err(ClaurstAcpTransportError::Uninitialized);
        }
        if session_id.is_empty() || prompt.trim().is_empty() || self.pending_prompt_id.is_some() {
            return Err(ClaurstAcpTransportError::InvalidSession);
        }
        let id = self.send_request(
            "session/prompt",
            json!({
                "sessionId": session_id,
                "prompt": [{"type": "text", "text": prompt}],
            }),
        )?;
        self.pending_prompt_id = Some(id);
        Ok(())
    }

    /// Reads at most `limit` queued/available frames; it never blocks waiting for provider output.
    pub(crate) fn drain(
        &mut self,
        limit: u16,
    ) -> Result<ClaurstAcpDrain, ClaurstAcpTransportError> {
        let limit = usize::from(limit.clamp(1, 64));
        let mut drain = ClaurstAcpDrain::default();
        while drain.facts.len() < limit {
            if let Some(fact) = self.queued.pop_front() {
                drain.facts.push(fact);
                continue;
            }
            let Some(frame) = self.read_frame()? else {
                break;
            };
            if let Some(terminal) = self.handle_frame(frame)? {
                drain.terminal = Some(terminal);
                break;
            }
        }
        Ok(drain)
    }

    fn send_request(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<u64, ClaurstAcpTransportError> {
        let id = self.next_request_id;
        self.next_request_id += 1;
        self.write(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        Ok(id)
    }

    fn wait_for_response(&mut self, request_id: u64) -> Result<Value, ClaurstAcpTransportError> {
        for _ in 0..MAX_HANDSHAKE_FRAMES {
            let frame = self
                .read_frame()?
                .ok_or(ClaurstAcpTransportError::HandshakeIncomplete)?;
            let value = parse_frame(&frame)?;
            if value.get("id") == Some(&json!(request_id))
                && (value.get("result").is_some() || value.get("error").is_some())
            {
                return value
                    .get("result")
                    .cloned()
                    .ok_or(ClaurstAcpTransportError::HandshakeIncomplete);
            }
            self.project(value)?;
        }
        Err(ClaurstAcpTransportError::HandshakeIncomplete)
    }

    fn read_frame(&mut self) -> Result<Option<Vec<u8>>, ClaurstAcpTransportError> {
        let frame = self
            .stdio
            .try_read_frame(MAX_ACP_FRAME_BYTES)
            .map_err(ClaurstAcpTransportError::Io)?;
        if frame
            .as_ref()
            .is_some_and(|value| value.len() > MAX_ACP_FRAME_BYTES)
        {
            return Err(ClaurstAcpTransportError::FrameTooLarge);
        }
        Ok(frame)
    }

    fn write(&mut self, value: Value) -> Result<(), ClaurstAcpTransportError> {
        let mut frame = serde_json::to_vec(&value).expect("JSON-RPC value serializes");
        frame.push(b'\n');
        self.stdio
            .write_frame(&frame)
            .map_err(ClaurstAcpTransportError::Io)
    }

    fn handle_frame(
        &mut self,
        frame: Vec<u8>,
    ) -> Result<Option<ClaurstAcpTerminal>, ClaurstAcpTransportError> {
        let value = parse_frame(&frame)?;
        if value.get("id").and_then(Value::as_u64) == self.pending_prompt_id
            && (value.get("result").is_some() || value.get("error").is_some())
        {
            self.pending_prompt_id = None;
            return Ok(Some(
                value
                    .get("error")
                    .is_some()
                    .then_some(ClaurstAcpTerminal::Failed)
                    .unwrap_or_else(|| prompt_terminal(&value)),
            ));
        }
        self.project(value)?;
        Ok(None)
    }

    fn project(&mut self, value: Value) -> Result<(), ClaurstAcpTransportError> {
        if value.get("method").and_then(Value::as_str) == Some("session/update") {
            if let Some(fact) = session_update_fact(value.get("params")) {
                self.queued.push_back(fact);
            }
        } else if value.get("method").and_then(Value::as_str) == Some("session/request_permission")
        {
            let id = value
                .get("id")
                .cloned()
                .ok_or(ClaurstAcpTransportError::InvalidFrame)?;
            // Permission policy composition is not connected yet, so fail closed rather than
            // letting a local tool run without a Gent decision.
            self.write(json!({"jsonrpc": "2.0", "id": id, "result": {"outcome": "cancelled"}}))?;
            self.queued.push_back(ClaurstAcpFact::Lifecycle(
                NormalizedLifecycleSignal::AttentionRequired,
            ));
        }
        Ok(())
    }
}

fn parse_frame(frame: &[u8]) -> Result<Value, ClaurstAcpTransportError> {
    serde_json::from_slice::<Value>(frame).map_err(|_| ClaurstAcpTransportError::InvalidFrame)
}

fn session_update_fact(params: Option<&Value>) -> Option<ClaurstAcpFact> {
    let update = params?.get("update")?;
    let text = update.get("content")?.get("text")?.as_str()?.to_owned();
    match update.get("sessionUpdate")?.as_str()? {
        "agent_message_chunk" => Some(ClaurstAcpFact::Event(NormalizedProviderEvent::Output {
            text,
            is_partial: true,
        })),
        "agent_thought_chunk" => Some(ClaurstAcpFact::Event(NormalizedProviderEvent::Thinking {
            text,
            is_partial: true,
        })),
        _ => None,
    }
}

fn prompt_terminal(value: &Value) -> ClaurstAcpTerminal {
    match value.pointer("/result/stopReason").and_then(Value::as_str) {
        Some("end_turn") | Some("max_tokens") | Some("max_turn_requests") => {
            ClaurstAcpTerminal::Completed
        }
        Some("cancelled") => ClaurstAcpTerminal::Interrupted,
        _ => ClaurstAcpTerminal::Failed,
    }
}

#[cfg(test)]
#[path = "claurst_acp_transport_tests.rs"]
mod tests;
