//! Public bounded inputs, consequences, and failures for a Codex session.

use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, PermissionMode,
};
use serde_json::{Value, json};

/// Provider turn fields derived exclusively from a durable Gent run selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexTurnOptions {
    model: Option<String>,
    effort: CodexTurnEffort,
    sandbox: CodexSandboxPolicy,
    approval: CodexApprovalPolicy,
    instruction: Option<&'static str>,
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
}

impl CodexTurnOptions {
    /// Converts a Gent-owned selection into bounded Codex turn options.
    ///
    pub fn from_selection(
        selection: &AgentChatSelection,
        working_directory: Option<&str>,
    ) -> Result<Self, CodexSessionError> {
        let permission_mode = match selection.mode {
            AgentChatMode::Ask => PermissionMode::Default,
            AgentChatMode::Plan => PermissionMode::Plan,
            AgentChatMode::Agent => PermissionMode::AutoAcceptEdits,
        };
        Self::from_selection_with_permissions(selection, working_directory, permission_mode)
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
        let sandbox =
            if permission_mode == PermissionMode::Bypass && selection.mode != AgentChatMode::Plan {
                CodexSandboxPolicy::DangerFullAccess
            } else {
                mode_sandbox
            };
        let approval = match permission_mode {
            PermissionMode::Default | PermissionMode::Plan => CodexApprovalPolicy::Untrusted,
            PermissionMode::AutoAcceptEdits | PermissionMode::Autonomous => {
                CodexApprovalPolicy::OnRequest
            }
            PermissionMode::Bypass => CodexApprovalPolicy::Never,
        };
        Ok(Self {
            model: (selection.model != "default").then(|| selection.model.clone()),
            effort,
            sandbox,
            approval,
            instruction: (selection.mode == AgentChatMode::Plan).then_some(
                "You are in Plan Mode. Do not make changes, run write commands, or apply patches. Inspect only as needed, then provide a complete, actionable plan and wait for user approval before implementation.",
            ),
            configured_append_system_prompt: None,
        })
    }

    pub fn summary(model: impl Into<String>) -> Result<Self, CodexSessionError> {
        let model = model.into();
        (!model.trim().is_empty() && !model.contains('\0'))
            .then_some(Self {
                model: Some(model),
                effort: CodexTurnEffort::Low,
                sandbox: CodexSandboxPolicy::ReadOnly,
                approval: CodexApprovalPolicy::Never,
                instruction: None,
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

pub(crate) fn turn_parameters(
    options: &CodexTurnOptions,
    thread_id: &str,
    prompt: &str,
    attachments: &[Value],
) -> Value {
    let sandbox_policy = match options.sandbox() {
        CodexSandboxPolicy::ReadOnly => json!({"type": "readOnly", "networkAccess": false}),
        CodexSandboxPolicy::WorkspaceWrite { writable_root } => json!({
            "type": "workspaceWrite",
            "writableRoots": writable_root.iter().collect::<Vec<_>>(),
            "networkAccess": false,
            "excludeTmpdirEnvVar": false,
            "excludeSlashTmp": false,
        }),
        CodexSandboxPolicy::DangerFullAccess => json!({"type": "dangerFullAccess"}),
    };
    let instruction = match (
        options.instruction,
        options.configured_append_system_prompt.as_deref(),
    ) {
        (Some(mode_text), Some(configured)) => Some(format!("{mode_text}\n\n{configured}")),
        (Some(text), None) => Some(text.to_owned()),
        (None, Some(text)) => Some(text.to_owned()),
        (None, None) => None,
    };
    let prompt = instruction.map_or_else(
        || prompt.into(),
        |instruction| format!("{instruction}\n\nUser request:\n{prompt}"),
    );
    let mut parameters = json!({
        "threadId": thread_id,
        "input": [{"type": "text", "text": prompt}],
        "effort": options.effort(),
        "approvalPolicy": options.approval(),
        "sandboxPolicy": sandbox_policy,
    });
    if let Some(model) = options.model() {
        parameters["model"] = Value::String(model.into());
    }
    if !attachments.is_empty() {
        let input = parameters["input"]
            .as_array_mut()
            .expect("fixed input array");
        input.extend(attachments.iter().cloned());
    }
    parameters
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
    pub mcp_servers: Option<serde_json::Value>,
}

/// Public consequences of one app-server frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodexSessionIngress {
    /// Frames that must be written in order before reading another provider response.
    Send(Vec<Vec<u8>>),
    /// The matching thread response established a usable native thread.
    Ready { thread_id: String },
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
    #[error("the Codex app-server has no live turn to interrupt")]
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
    #[error("the Codex turn notification and response disagreed")]
    TurnIdentityMismatch,
    #[error("the Codex app-server frame could not be encoded")]
    Serialization,
    #[error("the durable selection is not a Codex selection")]
    UnsupportedSelection,
    #[error("the Codex model must be non-empty and within the driver limit")]
    InvalidModel,
}

#[cfg(test)]
mod tests {
    use super::{CodexTurnOptions, turn_parameters};
    use gent_types::{
        AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, PermissionMode,
    };

    #[test]
    fn configured_append_system_prompt_composes_with_the_plan_mode_instruction() {
        let options = CodexTurnOptions::from_selection(&selection(AgentChatMode::Plan), None)
            .unwrap()
            .with_conversation_config(Some("Prefer terse replies.".into()), true);
        let parameters = turn_parameters(&options, "thread", "do the thing", &[]);
        let text = parameters["input"][0]["text"].as_str().unwrap();
        assert!(text.contains("Plan Mode"));
        assert!(text.contains("Prefer terse replies."));
        assert!(text.ends_with("User request:\ndo the thing"));
    }

    #[test]
    fn a_non_append_system_prompt_is_ignored_at_the_wire_layer() {
        let options = CodexTurnOptions::from_selection(&selection(AgentChatMode::Agent), None)
            .unwrap()
            .with_conversation_config(Some("Replace everything.".into()), false);
        let parameters = turn_parameters(&options, "thread", "do the thing", &[]);
        assert_eq!(parameters["input"][0]["text"], "do the thing");
    }

    fn selection(mode: AgentChatMode) -> AgentChatSelection {
        AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "gpt-5.6".into(),
            effort: AgentChatEffort::High,
            mode,
        }
    }

    #[test]
    fn summary_turn_never_requests_approval_or_network_access() {
        let options = CodexTurnOptions::summary("gpt-5.6-luna").unwrap();
        let parameters = turn_parameters(&options, "thread", "prompt", &[]);
        assert_eq!(parameters["approvalPolicy"], "never");
        assert_eq!(parameters["sandboxPolicy"]["type"], "readOnly");
        assert_eq!(parameters["sandboxPolicy"]["networkAccess"], false);
        assert_eq!(parameters["effort"], "low");
    }

    #[test]
    fn permission_posture_changes_approval_without_replacing_plan_intent() {
        let selection = AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "gpt-5.6".into(),
            effort: AgentChatEffort::Medium,
            mode: AgentChatMode::Plan,
        };
        let options = CodexTurnOptions::from_selection_with_permissions(
            &selection,
            Some("/workspace"),
            PermissionMode::Bypass,
        )
        .unwrap();
        let parameters = turn_parameters(&options, "thread", "prompt", &[]);
        assert_eq!(parameters["approvalPolicy"], "never");
        assert_eq!(parameters["sandboxPolicy"]["type"], "readOnly");
        assert!(
            parameters["input"][0]["text"]
                .as_str()
                .is_some_and(|text| text.contains("Plan Mode"))
        );
    }

    #[test]
    fn codex_preserves_its_highest_supported_effort_level() {
        let selection = AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "gpt-5.6-sol".into(),
            effort: AgentChatEffort::Ultra,
            mode: AgentChatMode::Agent,
        };
        let options = CodexTurnOptions::from_selection_with_permissions(
            &selection,
            Some("/workspace"),
            PermissionMode::Default,
        )
        .unwrap();
        assert_eq!(
            turn_parameters(&options, "thread", "prompt", &[])["effort"],
            "ultra"
        );
    }
}
