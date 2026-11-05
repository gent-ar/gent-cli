use std::{collections::VecDeque, path::Path, time::Duration};

use gent_ports::ClaurstPermissionReply;
use gent_types::{NormalizedLifecycleSignal, NormalizedProviderEvent, PermissionCategory};
use serde_json::{Value, json};

#[path = "claurst_acp_fixed_mcp.rs"]
mod fixed_mcp;
pub(crate) use fixed_mcp::mcp_roster_sha256;

const MAX_ACP_FRAME_BYTES: usize = gent_drivers::MAX_PROVIDER_FRAME_BYTES;
pub(crate) const OVERSIZED_FRAME_METHOD: &str = "gent/oversizedProviderFrame";
const MAX_HANDSHAKE_FRAMES: usize = 32;
const HANDSHAKE_TIMEOUT: Duration =
    crate::provider_launch_budget::launch_budget(Duration::from_secs(20));
const HANDSHAKE_RETRY_DELAY: Duration = Duration::from_millis(5);

#[path = "claurst_acp_stdio_contract.rs"]
mod stdio_contract;
pub(crate) use stdio_contract::ClaurstAcpStdio;

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
    pub(crate) input: Option<Value>,
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
    #[error("Claurst ACP rejected the request with JSON-RPC error {code}: {message}")]
    Rejected { code: i64, message: String },
    #[error("Claurst ACP returned an invalid session identifier")]
    InvalidSession,
    #[error("Claurst ACP I/O failed: {0}")]
    Io(String),
    #[error("Claurst ACP permission is not pending for this request")]
    InvalidPermission,
}

fn provider_spelling(workspace: &Path) -> std::path::PathBuf {
    #[cfg(windows)]
    if let Ok(ordinary) = crate::provider_egress::mcp_paths::ordinary_argument(workspace) {
        return ordinary;
    }
    workspace.to_path_buf()
}

pub(crate) struct ClaurstAcpTransport<S> {
    stdio: S,
    next_request_id: u64,
    initialized: bool,
    queued: VecDeque<ClaurstAcpFact>,
    queued_permissions: VecDeque<ClaurstAcpPermissionRequest>,
    open_tools: Vec<updates::OpenTool>,
    permission_claimed_ids: std::collections::BTreeSet<String>,
    permission_rpc_ids: std::collections::BTreeSet<String>,
    pending_prompt_id: Option<u64>,
    cancel_deadline: Option<std::time::Instant>,
    pending_terminal: Option<ClaurstAcpTerminal>,
    pending_permission: Option<PendingPermission>,
    held_permissions: VecDeque<(PendingPermission, ClaurstAcpPermissionRequest)>,
    assistant_output: String,
    mcp_servers: Vec<Value>,
    supports_images: bool,
}

struct PendingPermission {
    request_id: String,
    json_rpc_id: Value,
    allow_once_option: Option<String>,
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
            open_tools: Vec::new(),
            permission_claimed_ids: std::collections::BTreeSet::default(),
            permission_rpc_ids: std::collections::BTreeSet::default(),
            pending_prompt_id: None,
            cancel_deadline: None,
            pending_terminal: None,
            pending_permission: None,
            held_permissions: VecDeque::new(),
            assistant_output: String::new(),
            mcp_servers: Vec::new(),
            supports_images: false,
        }
    }

    pub(crate) fn with_mcp_servers(mut self, mcp_servers: Vec<Value>) -> Self {
        self.mcp_servers = mcp_servers;
        self
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
        let fixed = fixed_mcp::matches(self.stdio.fixed_mcp_roster_sha256(), &mcp_servers)?;
        self.initialize_with(fixed)?;
        let session_id = self.send_request(
            "session/new",
            json!({"cwd": provider_spelling(workspace), "mcpServers": if fixed{Vec::<Value>::new()}else{mcp_servers}}),
        )?;
        let response = self.wait_for_response(session_id)?;
        response
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && value.len() <= 256)
            .map(ToOwned::to_owned)
            .ok_or(ClaurstAcpTransportError::InvalidSession)
    }

    pub(crate) fn initialize(&mut self) -> Result<(), ClaurstAcpTransportError> {
        let fixed = fixed_mcp::matches(self.stdio.fixed_mcp_roster_sha256(), &self.mcp_servers)?;
        self.initialize_with(fixed)
    }

    fn initialize_with(&mut self, fixed: bool) -> Result<(), ClaurstAcpTransportError> {
        if !self.initialized {
            let initialize_id = self.send_request(
                "initialize",
                json!({
                    "protocolVersion": 1,
                    "clientCapabilities": {},
                    "_meta": {"gentPermissionBinding":1},
                    "clientInfo": {"name": "gent", "version": env!("CARGO_PKG_VERSION")},
                }),
            )?;
            let response = self.wait_for_response(initialize_id)?;
            fixed_mcp::check_sdk(&response, fixed)?;
            if response.pointer("/_meta/gentPermissionBinding") != Some(&json!(1)) {
                return Err(ClaurstAcpTransportError::InvalidFrame);
            }
            self.supports_images = response
                .pointer("/agentCapabilities/promptCapabilities/image")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            self.initialized = true;
        }
        Ok(())
    }

    pub(crate) fn prompt_content(
        &mut self,
        session_id: &str,
        content: Vec<Value>,
    ) -> Result<(), ClaurstAcpTransportError> {
        if !self.initialized {
            return Err(ClaurstAcpTransportError::Uninitialized);
        }
        if session_id.is_empty()
            || content.is_empty()
            || self.pending_prompt_id.is_some()
            || self.pending_permission.is_some()
            || !self.held_permissions.is_empty()
        {
            return Err(ClaurstAcpTransportError::InvalidSession);
        }
        let id = self.send_request(
            "session/prompt",
            json!({
                "sessionId": session_id,
                "prompt": content,
            }),
        )?;
        self.permission_claimed_ids.clear();
        self.permission_rpc_ids.clear();
        self.open_tools.clear();
        self.pending_prompt_id = Some(id);
        self.cancel_deadline = None;
        self.assistant_output.clear();
        Ok(())
    }

    pub(crate) fn respond_permission(
        &mut self,
        request_id: &str,
        reply: ClaurstPermissionReply,
    ) -> Result<(), ClaurstAcpTransportError> {
        if self
            .pending_permission
            .as_ref()
            .is_none_or(|pending| pending.request_id != request_id)
        {
            return Err(ClaurstAcpTransportError::InvalidPermission);
        }
        if matches!(reply, ClaurstPermissionReply::AllowOnce)
            && self
                .pending_permission
                .as_ref()
                .is_some_and(|pending| pending.allow_once_option.is_none())
        {
            return Err(ClaurstAcpTransportError::InvalidPermission);
        }
        let pending = self
            .pending_permission
            .take()
            .ok_or(ClaurstAcpTransportError::InvalidPermission)?;
        let result = match reply {
            ClaurstPermissionReply::AllowOnce => {
                json!({"outcome": {"outcome": "selected", "optionId": pending.allow_once_option}})
            }
            ClaurstPermissionReply::Deny => json!({"outcome": {"outcome": "cancelled"}}),
        };
        self.write(json!({"jsonrpc": "2.0", "id": pending.json_rpc_id, "result": result}))?;
        if let Some((next, request)) = self.held_permissions.pop_front() {
            self.pending_permission = Some(next);
            self.queued_permissions.push_back(request);
        }
        Ok(())
    }
}

#[path = "claurst_acp_transport_io.rs"]
mod io;

#[path = "claurst_acp_transport_updates.rs"]
mod updates;

#[cfg(test)]
#[path = "claurst_acp_transport_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "claurst_acp_transport_unanswered_tests.rs"]
mod unanswered_tests;

#[cfg(all(test, windows))]
#[path = "claurst_acp_transport_cwd_tests.rs"]
mod cwd_tests;

#[path = "claurst_acp_transport_authority.rs"]
mod authority;

#[cfg(test)]
#[path = "claurst_acp_permission_wire_tests.rs"]
mod exact_permission_wire_tests;

#[path = "claurst_acp_transport_prompt.rs"]
mod prompt;
