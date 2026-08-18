//! Public bounded inputs, consequences, and failures for a Codex session.

use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection};
use serde_json::{Value, json};

/// Provider turn fields derived exclusively from a durable Gent run selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexTurnOptions {
    model: String,
    effort: CodexTurnEffort,
    sandbox: CodexSandboxPolicy,
}

/// A bounded Codex effort value with no provider JSON at the daemon boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodexTurnEffort {
    Low,
    Medium,
    High,
}

/// The only sandbox postures currently derived from Gent interaction modes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodexSandboxPolicy {
    ReadOnly,
    WorkspaceWrite { writable_root: Option<String> },
}

impl CodexTurnOptions {
    /// Converts a Gent-owned selection into bounded Codex turn options.
    ///
    /// Approval stays `untrusted`; this type never carries a provider permission elevation.
    ///
    /// # Errors
    /// Rejects another provider or an empty or unbounded model identifier.
    pub fn from_selection(
        selection: &AgentChatSelection,
        working_directory: Option<&str>,
    ) -> Result<Self, CodexSessionError> {
        if selection.provider != AgentChatProvider::Codex {
            return Err(CodexSessionError::UnsupportedSelection);
        }
        if selection.validate().is_err() {
            return Err(CodexSessionError::InvalidModel);
        }
        let effort = match selection.effort {
            AgentChatEffort::Low => CodexTurnEffort::Low,
            AgentChatEffort::Medium => CodexTurnEffort::Medium,
            AgentChatEffort::High => CodexTurnEffort::High,
        };
        let sandbox = match selection.mode {
            AgentChatMode::Ask | AgentChatMode::Plan => CodexSandboxPolicy::ReadOnly,
            AgentChatMode::Agent => CodexSandboxPolicy::WorkspaceWrite {
                writable_root: working_directory.map(str::to_owned),
            },
        };
        Ok(Self {
            model: selection.model.clone(),
            effort,
            sandbox,
        })
    }

    pub(crate) fn model(&self) -> &str {
        &self.model
    }

    pub(crate) fn effort(&self) -> &'static str {
        match self.effort {
            CodexTurnEffort::Low => "low",
            CodexTurnEffort::Medium => "medium",
            CodexTurnEffort::High => "high",
        }
    }

    pub(crate) fn sandbox(&self) -> &CodexSandboxPolicy {
        &self.sandbox
    }
}

pub(crate) fn turn_parameters(options: &CodexTurnOptions, thread_id: &str, prompt: &str) -> Value {
    let sandbox_policy = match options.sandbox() {
        CodexSandboxPolicy::ReadOnly => json!({"type": "readOnly", "networkAccess": false}),
        CodexSandboxPolicy::WorkspaceWrite { writable_root } => json!({
            "type": "workspaceWrite",
            "writableRoots": writable_root.iter().collect::<Vec<_>>(),
            "networkAccess": false,
            "excludeTmpdirEnvVar": false,
            "excludeSlashTmp": false,
        }),
    };
    json!({
        "threadId": thread_id,
        "input": [{"type": "text", "text": prompt}],
        "model": options.model(),
        "effort": options.effort(),
        "approvalPolicy": "untrusted",
        "sandboxPolicy": sandbox_policy,
    })
}

/// Inputs fixed before the Codex app-server initialization request is sent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexSessionConfig {
    /// Optional provider working directory for the thread request.
    pub working_directory: Option<String>,
    /// The exact previously recorded native thread to reattach, if any.
    pub resume_thread_id: Option<String>,
    /// Bounded turn settings resolved from the durable Gent run selection.
    pub turn_options: CodexTurnOptions,
}

/// Public consequences of one app-server frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodexSessionIngress {
    /// Frames that must be written in order before reading another provider response.
    Send(Vec<Vec<u8>>),
    /// The matching thread response established a usable native thread.
    Ready,
    /// The matching turn response established a live native turn.
    TurnStarted,
    /// The live native turn ended.
    TurnEnded,
    /// The frame did not affect this session.
    Ignored,
}

/// Controlled failures for the bounded Codex app-server handshake.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CodexSessionError {
    #[error("the Codex working directory must be non-empty and within the driver limit")]
    InvalidWorkingDirectory,
    #[error("the Codex native thread identity must be non-empty and within the driver limit")]
    InvalidThreadId,
    #[error("a Codex user prompt must be non-empty and within the driver limit")]
    InvalidPrompt,
    #[error("the Codex app-server thread is not ready for a user turn")]
    ThreadNotReady,
    #[error("the Codex app-server already has a live or pending user turn")]
    TurnAlreadyActive,
    #[error("the Codex app-server session previously failed")]
    SessionFailed,
    #[error("the Codex app-server request identifier space is exhausted")]
    RequestIdExhausted,
    #[error("the Codex app-server returned a malformed response")]
    MalformedResponse,
    #[error("the Codex app-server returned an uncorrelated response")]
    UncorrelatedResponse,
    #[error("the Codex app-server rejected a correlated request")]
    RequestRejected,
    #[error("the resumed Codex thread did not match the exact recorded identity")]
    ResumedThreadMismatch,
    #[error("the Codex turn notification and response disagreed")]
    TurnIdentityMismatch,
    #[error("the Codex app-server frame could not be encoded")]
    Serialization,
    #[error("the durable selection is not a Codex selection")]
    UnsupportedSelection,
    #[error("the Codex model must be non-empty and within the driver limit")]
    InvalidModel,
}
