use super::prompt::{prompt_failure, prompt_terminal};
use super::{
    ClaurstAcpFact, ClaurstAcpPermissionRequest, ClaurstAcpStdio, ClaurstAcpTerminal,
    ClaurstAcpTransport, ClaurstAcpTransportError, HANDSHAKE_RETRY_DELAY, HANDSHAKE_TIMEOUT,
    MAX_ACP_FRAME_BYTES, PendingPermission,
};
use gent_types::PermissionCategory;
use serde_json::{Value, json};
use std::time::Instant;
#[path = "claurst_acp_mcp_call.rs"]
mod mcp_call;

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
            if value.get("id") == Some(&json!(request_id)) {
                if let Some(error) = value.get("error") {
                    return Err(ClaurstAcpTransportError::Rejected {
                        code: error
                            .get("code")
                            .and_then(Value::as_i64)
                            .unwrap_or_default(),
                        message: error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .chars()
                            .take(1_024)
                            .collect(),
                    });
                }
                if let Some(result) = value.get("result") {
                    return Ok(result.clone());
                }
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
                prompt_terminal(&value, &self.assistant_output)
            };
            if terminal == ClaurstAcpTerminal::Failed && value.get("error").is_none() {
                let (classification, message) = prompt_failure(&value, &self.assistant_output);
                self.queued.push_back(ClaurstAcpFact::Event(
                    gent_types::NormalizedProviderEvent::ProviderFailure {
                        classification,
                        message: message.into(),
                    },
                ));
            }
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
        if value.get("method").and_then(Value::as_str) == Some(super::OVERSIZED_FRAME_METHOD) {
            self.queued.push_back(ClaurstAcpFact::Event(
                gent_types::NormalizedProviderEvent::TransportDiagnostic {
                    classification: gent_types::OVERSIZED_PROVIDER_FRAME_DIAGNOSTIC.into(),
                },
            ));
        } else if value.get("method").and_then(Value::as_str) == Some("session/update") {
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

        let tool_call = value
            .pointer("/params/toolCall")
            .ok_or(ClaurstAcpTransportError::InvalidFrame)?;
        let tool_use_id = tool_call
            .get("toolCallId")
            .and_then(Value::as_str)
            .filter(|v| !v.trim().is_empty())
            .map(str::to_owned)
            .ok_or(ClaurstAcpTransportError::InvalidFrame)?;
        let direct =
            mcp_call::read(tool_call).map_err(|_| ClaurstAcpTransportError::InvalidFrame)?;
        let request_id =
            permission_request_id(&id).ok_or(ClaurstAcpTransportError::InvalidFrame)?;
        // Wire uniqueness uses the original JSON-RPC ID across every tool kind.
        // API identifiers below independently preserve the same string/number type.
        let rpc_identity =
            serde_json::to_string(&id).map_err(|_| ClaurstAcpTransportError::InvalidFrame)?;
        if self.permission_rpc_ids.len() >= 4096 || self.permission_rpc_ids.contains(&rpc_identity)
        {
            return Err(ClaurstAcpTransportError::InvalidFrame);
        }
        let category = if direct.is_some() {
            PermissionCategory::Provider
        } else {
            permission_category(tool_call)
        };
        let options = value
            .pointer("/params/options")
            .map(|options| {
                options
                    .as_array()
                    .ok_or(ClaurstAcpTransportError::InvalidFrame)
            })
            .transpose()?;
        let mut allow_once_id = None;
        if let Some(options) = options {
            if options.len() > 16 {
                return Err(ClaurstAcpTransportError::InvalidFrame);
            }
            let mut ids = std::collections::BTreeSet::new();
            for option in options {
                let option_id = option
                    .get("optionId")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty() && id.len() <= 512)
                    .ok_or(ClaurstAcpTransportError::InvalidFrame)?;
                if !ids.insert(option_id) {
                    return Err(ClaurstAcpTransportError::InvalidFrame);
                }
                if option.get("kind").and_then(Value::as_str) == Some("allow_once") {
                    if allow_once_id.is_some() {
                        return Err(ClaurstAcpTransportError::InvalidFrame);
                    }
                    allow_once_id = Some(option_id.to_owned());
                }
            }
        }
        let tool = self
            .claim_permission_tool(tool_call)
            .ok_or(ClaurstAcpTransportError::InvalidFrame)?;
        let pending = PendingPermission {
            request_id: request_id.clone(),
            json_rpc_id: id,
            allow_once_option: allow_once_id,
        };
        let request = if let Some((tool_name, input)) = direct {
            if tool_use_id.len() > 256
                || tool.tool_use_id != tool_use_id
                || tool.tool_name != tool_name
                || tool.input.as_ref() != Some(&input)
            {
                return Err(ClaurstAcpTransportError::InvalidFrame);
            }
            ClaurstAcpPermissionRequest {
                request_id,
                tool_use_id,
                tool_name,
                category: PermissionCategory::Provider,
                input: Some(input),
            }
        } else {
            ClaurstAcpPermissionRequest {
                request_id,
                tool_use_id: tool.tool_use_id,
                tool_name: tool.tool_name,
                category,
                input: tool.input,
            }
        };
        self.permission_rpc_ids.insert(rpc_identity);
        if self.pending_permission.is_some() {
            self.held_permissions.push_back((pending, request));
        } else {
            self.pending_permission = Some(pending);
            self.queued_permissions.push_back(request);
        }
        Ok(())
    }
}
// A single injective encoding for ALL callback kinds. The string payload is
// copied without normalization; a string can never occupy the numeric branch.
// Preserve the prior 128-byte raw string bound; internal IDs are at most138 bytes.
pub(super) fn permission_request_id(id: &Value) -> Option<String> {
    match id {
        Value::String(v) if !v.trim().is_empty() && v.len() <= 128 => {
            Some(format!("claurst:s:{v}"))
        }
        Value::Number(v) if v.is_i64() || v.is_u64() => Some(format!("claurst:n:{v}")),
        _ => None,
    }
}
fn permission_category(call: &Value) -> PermissionCategory {
    match call.get("kind").and_then(Value::as_str) {
        Some("read" | "search") => PermissionCategory::Read,
        Some("edit" | "delete" | "move") => PermissionCategory::Edit,
        Some("execute") => PermissionCategory::Command,
        Some("fetch") => PermissionCategory::Network,
        _ => PermissionCategory::Provider,
    }
}
fn parse_frame(frame: &[u8]) -> Result<Value, ClaurstAcpTransportError> {
    serde_json::from_slice(frame).map_err(|_| ClaurstAcpTransportError::InvalidFrame)
}
pub(super) fn unanswered_stop_reason(output: &str) -> Option<&str> {
    output
        .trim()
        .strip_prefix("(no response — model ended the turn with stop_reason \"")?
        .strip_suffix("\")")
}
