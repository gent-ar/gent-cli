use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
    time::Duration,
};

use gent_ports::ClaurstPermissionReply;
use gent_types::{NormalizedLifecycleSignal, NormalizedProviderEvent, PermissionCategory};
use serde_json::{Value, json};

const MAX_ACP_FRAME_BYTES: usize = 256 * 1024;
const MAX_HANDSHAKE_FRAMES: usize = 32;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const HANDSHAKE_RETRY_DELAY: Duration = Duration::from_millis(5);

pub(crate) trait ClaurstAcpStdio {
    fn write_frame(&mut self, frame: &[u8]) -> Result<(), String>;
    fn try_read_frame(&mut self, maximum_bytes: usize) -> Result<Option<Vec<u8>>, String>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClaurstAcpTerminal {
    Completed,
    Interrupted,
    Failed,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ClaurstAcpDrain {
    pub(crate) facts: Vec<ClaurstAcpFact>,
    pub(crate) permissions: Vec<ClaurstAcpPermissionRequest>,
    pub(crate) terminal: Option<ClaurstAcpTerminal>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClaurstAcpPermissionRequest {
    pub(crate) request_id: String,
    pub(crate) tool_use_id: String,
    pub(crate) tool_name: String,
    pub(crate) category: PermissionCategory,
}

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
    #[error("Claurst ACP permission is not pending for this request")]
    InvalidPermission,
}

pub(crate) struct ClaurstAcpTransport<S> {
    stdio: S,
    next_request_id: u64,
    initialized: bool,
    queued: VecDeque<ClaurstAcpFact>,
    queued_permissions: VecDeque<ClaurstAcpPermissionRequest>,
    tool_names: BTreeMap<String, String>,
    pending_prompt_id: Option<u64>,
    pending_terminal: Option<ClaurstAcpTerminal>,
    pending_permission: Option<PendingPermission>,
    assistant_output: String,
    mcp_servers: Vec<Value>,
    supports_images: bool,
}

struct PendingPermission {
    request_id: String,
    json_rpc_id: Value,
}

impl<S: ClaurstAcpStdio> ClaurstAcpTransport<S> {
    #[must_use]
    pub(crate) fn new(stdio: S) -> Self {
        Self {
            stdio,
            next_request_id: 1,
            initialized: false,
            queued: VecDeque::new(),
            queued_permissions: VecDeque::new(),
            tool_names: BTreeMap::new(),
            pending_prompt_id: None,
            pending_terminal: None,
            pending_permission: None,
            assistant_output: String::new(),
            mcp_servers: Vec::new(),
            supports_images: false,
        }
    }

    pub(crate) fn with_mcp_servers(mut self, mcp_servers: Vec<Value>) -> Self {
        self.mcp_servers = mcp_servers;
        self
    }

    pub(crate) fn supports_images(&self) -> bool {
        self.supports_images
    }

    pub(crate) fn is_idle(&self) -> bool {
        self.pending_prompt_id.is_none()
    }

    pub(crate) fn initialize_session(
        &mut self,
        workspace: &Path,
    ) -> Result<String, ClaurstAcpTransportError> {
        self.initialize_session_with_mcp(workspace, self.mcp_servers.clone())
    }

    pub(crate) fn initialize_session_with_mcp(
        &mut self,
        workspace: &Path,
        mcp_servers: Vec<Value>,
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
            let response = self.wait_for_response(initialize_id)?;
            self.supports_images = response
                .pointer("/agentCapabilities/promptCapabilities/image")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            self.initialized = true;
        }
        let session_id = self.send_request(
            "session/new",
            json!({"cwd": workspace, "mcpServers": mcp_servers}),
        )?;
        let response = self.wait_for_response(session_id)?;
        response
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && value.len() <= 256)
            .map(ToOwned::to_owned)
            .ok_or(ClaurstAcpTransportError::InvalidSession)
    }

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
        self.prompt_content(session_id, vec![json!({"type": "text", "text": prompt})])
    }

    pub(crate) fn prompt_content(
        &mut self,
        session_id: &str,
        content: Vec<Value>,
    ) -> Result<(), ClaurstAcpTransportError> {
        if !self.initialized {
            return Err(ClaurstAcpTransportError::Uninitialized);
        }
        if session_id.is_empty() || content.is_empty() || self.pending_prompt_id.is_some() {
            return Err(ClaurstAcpTransportError::InvalidSession);
        }
        let id = self.send_request(
            "session/prompt",
            json!({
                "sessionId": session_id,
                "prompt": content,
            }),
        )?;
        self.pending_prompt_id = Some(id);
        self.assistant_output.clear();
        Ok(())
    }

    pub(crate) fn cancel(&mut self, session_id: &str) -> Result<(), ClaurstAcpTransportError> {
        if !self.initialized || session_id.is_empty() {
            return Err(ClaurstAcpTransportError::InvalidSession);
        }
        self.write(json!({
            "jsonrpc": "2.0",
            "method": "session/cancel",
            "params": {"sessionId": session_id},
        }))
    }

    pub(crate) fn drain(
        &mut self,
        limit: u16,
    ) -> Result<ClaurstAcpDrain, ClaurstAcpTransportError> {
        let limit = usize::from(limit.clamp(1, 64));
        let mut drain = ClaurstAcpDrain::default();
        while drain.facts.len() < limit && drain.permissions.is_empty() {
            if let Some(fact) = self.queued.pop_front() {
                drain.facts.push(fact);
                continue;
            }
            if let Some(permission) = self.queued_permissions.pop_front() {
                drain.permissions.push(permission);
                continue;
            }
            if let Some(terminal) = self.pending_terminal.take() {
                drain.terminal = Some(terminal);
                break;
            }
            let Some(frame) = self.read_frame()? else {
                break;
            };
            if let Some(terminal) = self.handle_frame(frame)? {
                self.pending_terminal = Some(terminal);
            }
        }
        Ok(drain)
    }

    pub(crate) fn respond_permission(
        &mut self,
        request_id: &str,
        reply: ClaurstPermissionReply,
    ) -> Result<(), ClaurstAcpTransportError> {
        let pending = self
            .pending_permission
            .take()
            .filter(|pending| pending.request_id == request_id)
            .ok_or(ClaurstAcpTransportError::InvalidPermission)?;
        let result = match reply {
            ClaurstPermissionReply::AllowOnce => {
                json!({"outcome": {"outcome": "selected", "optionId": "allow_once"}})
            }
            ClaurstPermissionReply::Deny => json!({"outcome": {"outcome": "cancelled"}}),
        };
        self.write(json!({"jsonrpc": "2.0", "id": pending.json_rpc_id, "result": result}))
    }
}

#[path = "claurst_acp_transport_io.rs"]
mod io;

#[path = "claurst_acp_transport_updates.rs"]
mod updates;

#[cfg(test)]
#[path = "claurst_acp_transport_tests.rs"]
mod tests;
