use super::{
    ClaurstAcpFact, ClaurstAcpPermissionRequest, ClaurstAcpStdio, ClaurstAcpTerminal,
    ClaurstAcpTransport, ClaurstAcpTransportError, HANDSHAKE_RETRY_DELAY, HANDSHAKE_TIMEOUT,
    MAX_ACP_FRAME_BYTES, PendingPermission,
};
use gent_types::PermissionCategory;
use serde_json::{Value, json};
use std::time::Instant;

impl<S: ClaurstAcpStdio> ClaurstAcpTransport<S> {
    pub(super) fn send_request(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<u64, ClaurstAcpTransportError> {
        let id = self.next_request_id;
        self.next_request_id += 1;
        self.write(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        Ok(id)
    }
    pub(super) fn wait_for_response(
        &mut self,
        request_id: u64,
    ) -> Result<Value, ClaurstAcpTransportError> {
        let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
        let mut inspected = 0;
        while Instant::now() < deadline && inspected < super::MAX_HANDSHAKE_FRAMES {
            let Some(frame) = self.read_frame()? else {
                std::thread::sleep(HANDSHAKE_RETRY_DELAY);
                continue;
            };
            inspected += 1;
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
    pub(super) fn read_frame(&mut self) -> Result<Option<Vec<u8>>, ClaurstAcpTransportError> {
        let frame = self
            .stdio
            .try_read_frame(MAX_ACP_FRAME_BYTES)
            .map_err(ClaurstAcpTransportError::Io)?;
        if frame
            .as_ref()
            .is_some_and(|v| v.len() > MAX_ACP_FRAME_BYTES)
        {
            return Err(ClaurstAcpTransportError::FrameTooLarge);
        }
        Ok(frame)
    }
    pub(super) fn write(&mut self, value: Value) -> Result<(), ClaurstAcpTransportError> {
        let mut frame = serde_json::to_vec(&value).expect("JSON-RPC value serializes");
        frame.push(b'\n');
        self.stdio
            .write_frame(&frame)
            .map_err(ClaurstAcpTransportError::Io)
    }
    pub(super) fn handle_frame(
        &mut self,
        frame: Vec<u8>,
    ) -> Result<Option<ClaurstAcpTerminal>, ClaurstAcpTransportError> {
        let value = parse_frame(&frame)?;
        if value.get("id").and_then(Value::as_u64) == self.pending_prompt_id
            && (value.get("result").is_some() || value.get("error").is_some())
        {
            self.pending_prompt_id = None;
            let terminal = if let Some(error) = value.get("error") {
                self.queued.push_back(ClaurstAcpFact::Event(
                    gent_types::NormalizedProviderEvent::Output {
                        text: format!(
                            "Claurst ACP prompt failed: {}",
                            serde_json::to_string(error)
                                .map_err(|_| ClaurstAcpTransportError::InvalidFrame)?
                        ),
                        is_partial: false,
                    },
                ));
                ClaurstAcpTerminal::Failed
            } else {
                prompt_terminal(&value)
            };
            if terminal == ClaurstAcpTerminal::Completed && !self.assistant_output.is_empty() {
                self.queued.push_back(ClaurstAcpFact::Event(
                    gent_types::NormalizedProviderEvent::Output {
                        text: self.assistant_output.clone(),
                        is_partial: false,
                    },
                ));
            }
            return Ok(Some(terminal));
        }
        self.project(value)?;
        Ok(None)
    }
    fn project(&mut self, value: Value) -> Result<(), ClaurstAcpTransportError> {
        if value.get("method").and_then(Value::as_str) == Some("session/update") {
            if let Some(fact) = self.session_update_fact(value.get("params")) {
                self.queued.push_back(fact);
            }
        } else if value.get("method").and_then(Value::as_str) == Some("session/request_permission")
        {
            self.request_permission(value)?;
        }
        Ok(())
    }
    fn request_permission(&mut self, value: Value) -> Result<(), ClaurstAcpTransportError> {
        let id = value
            .get("id")
            .cloned()
            .ok_or(ClaurstAcpTransportError::InvalidFrame)?;
        if self.pending_permission.is_some() {
            return self.write(
                json!({"jsonrpc":"2.0","id":id,"result":{"outcome":{"outcome":"cancelled"}}}),
            );
        }
        let request_id = json_rpc_request_id(&id).ok_or(ClaurstAcpTransportError::InvalidFrame)?;
        let tool_call = value
            .pointer("/params/toolCall")
            .ok_or(ClaurstAcpTransportError::InvalidFrame)?;
        let tool_use_id = tool_call
            .get("toolCallId")
            .and_then(Value::as_str)
            .filter(|v| !v.trim().is_empty())
            .map(str::to_owned)
            .ok_or(ClaurstAcpTransportError::InvalidFrame)?;
        let tool_name = safe_permission_tool_name(tool_call);
        let category = if tool_name.eq_ignore_ascii_case("bash")
            || tool_name.eq_ignore_ascii_case("execute")
        {
            PermissionCategory::Command
        } else {
            PermissionCategory::Provider
        };
        self.pending_permission = Some(PendingPermission {
            request_id: request_id.clone(),
            json_rpc_id: id,
        });
        self.queued_permissions
            .push_back(ClaurstAcpPermissionRequest {
                request_id,
                tool_use_id,
                tool_name,
                category,
            });
        Ok(())
    }
}
fn json_rpc_request_id(id: &Value) -> Option<String> {
    match id {
        Value::String(v) if !v.trim().is_empty() && v.len() <= 128 => Some(v.clone()),
        Value::Number(v) => Some(v.to_string()),
        _ => None,
    }
}
fn safe_permission_tool_name(call: &Value) -> String {
    super::updates::safe_tool_name(
        call.get("title")
            .and_then(Value::as_str)
            .or_else(|| call.get("kind").and_then(Value::as_str)),
    )
}
fn parse_frame(frame: &[u8]) -> Result<Value, ClaurstAcpTransportError> {
    serde_json::from_slice(frame).map_err(|_| ClaurstAcpTransportError::InvalidFrame)
}
fn prompt_terminal(value: &Value) -> ClaurstAcpTerminal {
    match value.pointer("/result/stopReason").and_then(Value::as_str) {
        Some("end_turn" | "max_tokens" | "max_turn_requests") => ClaurstAcpTerminal::Completed,
        Some("cancelled") => ClaurstAcpTerminal::Interrupted,
        _ => ClaurstAcpTerminal::Failed,
    }
}
