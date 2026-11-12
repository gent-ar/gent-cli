//! Public bounded inputs, consequences, and failures for a Codex session.

use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, PermissionMode,
};

/// Provider turn fields derived exclusively from a durable Gent run selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexTurnOptions {
    model: Option<String>,
    effort: CodexTurnEffort,
    sandbox: CodexSandboxPolicy,
    approval: CodexApprovalPolicy,
    plan: bool,
    configured_append_system_prompt: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CodexApprovalPolicy {
    Untrusted,
    OnRequest,
    Never,
}

/// A bounded Codex effort value with no provider JSON at the daemon boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodexTurnEffort {
    Low,
    Medium,
    High,
    XHigh,
    Max,
    Ultra,
}

/// The only sandbox postures currently derived from Gent interaction modes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodexSandboxPolicy {
    ReadOnly,
    WorkspaceWrite { writable_root: Option<String> },
    DangerFullAccess,
    ExternalSandbox,
}

impl CodexTurnOptions {
    /// Converts a Gent-owned selection into bounded Codex turn options.
    ///
    pub fn from_selection(
        selection: &AgentChatSelection,
        working_directory: Option<&str>,
    ) -> Result<Self, CodexSessionError> {
        Self::from_selection_with_permissions(
            selection,
            working_directory,
            PermissionMode::AskEveryTime,
        )
    }

    pub fn from_selection_with_permissions(
        selection: &AgentChatSelection,
        working_directory: Option<&str>,
        permission_mode: PermissionMode,
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
            AgentChatEffort::XHigh => CodexTurnEffort::XHigh,
            AgentChatEffort::Max => CodexTurnEffort::Max,
            AgentChatEffort::Ultra => CodexTurnEffort::Ultra,
        };
        let mode_sandbox = match selection.mode {
            AgentChatMode::Ask | AgentChatMode::Plan => CodexSandboxPolicy::ReadOnly,
            AgentChatMode::Agent => CodexSandboxPolicy::WorkspaceWrite {
                writable_root: working_directory.map(str::to_owned),
            },
        };
        let sandbox = if permission_mode == PermissionMode::Bypass
            && selection.mode == AgentChatMode::Agent
        {
            CodexSandboxPolicy::DangerFullAccess
        } else {
            mode_sandbox
        };
        let approval = match (selection.mode, permission_mode) {
            (AgentChatMode::Plan, _) => CodexApprovalPolicy::Never,
            (AgentChatMode::Ask, _) | (_, PermissionMode::Autonomous) => {
                CodexApprovalPolicy::OnRequest
            }
            (_, PermissionMode::Bypass) => CodexApprovalPolicy::Never,
            (_, PermissionMode::AskEveryTime | PermissionMode::AutoAcceptEdits) => {
                CodexApprovalPolicy::Untrusted
            }
        };
        Ok(Self {
            model: (selection.model != "default").then(|| selection.model.clone()),
            effort,
            sandbox,
            approval,
            plan: selection.mode == AgentChatMode::Plan,
            configured_append_system_prompt: None,
        })
    }

    pub fn summary(model: impl Into<String>) -> Result<Self, CodexSessionError> {
        let model = model.into();
        (!model.trim().is_empty() && !model.contains('\0'))
            .then_some(Self {
                model: (model != "default").then_some(model),
                effort: CodexTurnEffort::Low,
                sandbox: CodexSandboxPolicy::ReadOnly,
                approval: CodexApprovalPolicy::Never,
                plan: false,
                configured_append_system_prompt: None,
            })
            .ok_or(CodexSessionError::InvalidModel)
    }

    /// Applies the append-mode half of a conversation's durable advanced launch configuration.
    ///
    /// Codex's app-server surface exposes no raw system-prompt override channel and no per-turn
    /// cap or tool denylist equivalent to Claude's `--max-turns`/`--disallowedTools` — only an
    /// append-style prompt prefix, the same mechanism this driver already uses for the Plan-mode
    /// instruction. A non-append `system_prompt` is silently ignored here; the daemon reports it
    /// as unsupported for this provider through `agent-chat-conversation-config-v1` instead.
    #[must_use]
    pub fn with_conversation_config(
        mut self,
        system_prompt: Option<String>,
        append_system_prompt: bool,
    ) -> Self {
        self.configured_append_system_prompt = system_prompt.filter(|_| append_system_prompt);
        self
    }

    pub(crate) fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    pub(super) fn resolve_provider_default(
        mut self,
        model: String,
    ) -> Result<Self, CodexSessionError> {
        if self.model.is_some()
            || model.is_empty()
            || model.len() > 512
            || model.contains('\0')
            || model == "default"
        {
            return Err(CodexSessionError::ProviderDefaultUnavailable);
        }
        self.model = Some(model);
        Ok(self)
    }

    pub(crate) fn effort(&self) -> &'static str {
        match self.effort {
            CodexTurnEffort::Low => "low",
            CodexTurnEffort::Medium => "medium",
            CodexTurnEffort::High => "high",
            CodexTurnEffort::XHigh => "xhigh",
            CodexTurnEffort::Max => "max",
            CodexTurnEffort::Ultra => "ultra",
        }
    }

    pub(super) fn bind_external_sandbox(&mut self) {
        self.sandbox = CodexSandboxPolicy::ExternalSandbox;
    }

    pub(crate) fn sandbox(&self) -> &CodexSandboxPolicy {
        &self.sandbox
    }

    fn approval(&self) -> &'static str {
        match self.approval {
            CodexApprovalPolicy::Untrusted => "untrusted",
            CodexApprovalPolicy::OnRequest => "on-request",
            CodexApprovalPolicy::Never => "never",
        }
    }
}

#[path = "turn_parameters.rs"]
mod parameters;
pub(crate) use parameters::turn_parameters;

/// Inputs fixed before the Codex app-server initialization request is sent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexSessionConfig {
    /// Optional provider working directory for the thread request.
    pub working_directory: Option<String>,
    /// The exact previously recorded native thread to reattach, if any.
    pub resume_thread_id: Option<String>,
    /// Bounded turn settings resolved from the durable Gent run selection.
    pub turn_options: CodexTurnOptions,
    pub mcp_servers: Option<serde_json::Value>,
}

/// Public consequences of one app-server frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodexSessionIngress {
    /// Frames that must be written in order before reading another provider response.
    Send(Vec<Vec<u8>>),
    /// The matching thread response established a usable native thread.
    Ready {
        thread_id: String,
    },
    /// The matching turn response established a live native turn.
    TurnStarted,
    /// The live native turn ended.
    TurnEnded,
    /// The frame did not affect this session.
    Ignored,
    Steer(super::CodexSteerOutcome),
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
    #[error("the Codex app-server has no live turn")]
    TurnNotActive,
    #[error("the Codex app-server already has a pending interrupt request")]
    InterruptAlreadyRequested,
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
    #[error("the Codex app-server no longer has the resumed thread")]
    ResumedThreadUnavailable,
    #[error("the Codex turn notification and response disagreed")]
    TurnIdentityMismatch,
    #[error("the Codex app-server frame could not be encoded")]
    Serialization,
    #[error("the durable selection is not a Codex selection")]
    UnsupportedSelection,
    #[error("the Codex model must be non-empty and within the driver limit")]
    InvalidModel,
    #[error("Codex did not publish one unambiguous available default model")]
    ProviderDefaultUnavailable,
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
