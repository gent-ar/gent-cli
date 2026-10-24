//! Bounded Claude launch fields derived from a durable Gent run selection.

use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, PermissionMode,
};

/// Claude launch fields derived only from an immutable Gent run selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaudeTurnOptions {
    model: String,
    mode: ClaudeTurnMode,
    instruction: Option<&'static str>,
    system_prompt: Option<String>,
    append_system_prompt: bool,
    max_turns: Option<u32>,
    thinking_enabled: Option<bool>,
    disallowed_tools: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClaudeTurnMode {
    Chat(ClaudePermissionMode, AgentChatEffort),
    Summary,
}

/// The complete set of Claude permission modes Gent may request for a chat turn.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaudePermissionMode {
    Plan,
    Manual,
    AcceptEdits,
    Auto,
    Bypass,
}

impl ClaudeTurnOptions {
    pub fn from_selection(selection: &AgentChatSelection) -> Result<Self, ClaudeTurnOptionsError> {
        Self::from_selection_with_permissions(selection, PermissionMode::AskEveryTime)
    }

    pub fn from_selection_with_permissions(
        selection: &AgentChatSelection,
        permission_mode: PermissionMode,
    ) -> Result<Self, ClaudeTurnOptionsError> {
        if selection.provider != AgentChatProvider::Claude {
            return Err(ClaudeTurnOptionsError::UnsupportedSelection);
        }
        if selection.validate().is_err() {
            return Err(ClaudeTurnOptionsError::InvalidModel);
        }
        let permission_mode = if selection.mode == AgentChatMode::Plan {
            ClaudePermissionMode::Plan
        } else {
            ClaudePermissionMode::from(permission_mode)
        };
        Ok(Self {
            model: selection.model.clone(),
            mode: ClaudeTurnMode::Chat(permission_mode, selection.effort),
            instruction: mode_instruction(selection.mode),
            system_prompt: None,
            append_system_prompt: false,
            max_turns: None,
            thinking_enabled: None,
            disallowed_tools: Vec::new(),
        })
    }

    #[must_use]
    pub const fn plans_for_review(&self) -> bool {
        matches!(
            self.mode,
            ClaudeTurnMode::Chat(ClaudePermissionMode::Plan, _)
        )
    }

    pub fn summary(model: impl Into<String>) -> Result<Self, ClaudeTurnOptionsError> {
        let model = model.into();
        (!model.trim().is_empty() && !model.contains('\0'))
            .then_some(Self {
                model,
                mode: ClaudeTurnMode::Summary,
                instruction: None,
                system_prompt: None,
                append_system_prompt: false,
                max_turns: None,
                thinking_enabled: None,
                disallowed_tools: Vec::new(),
            })
            .ok_or(ClaudeTurnOptionsError::InvalidModel)
    }

    /// Applies a conversation's durable advanced launch configuration.
    ///
    /// `system_prompt` either appends onto this turn's existing instruction (mode text plus the
    /// configured text, composed into one `--append-system-prompt` value) or, when
    /// `append_system_prompt` is false, replaces Claude's default prompt via `--system-prompt`
    /// while the mode instruction still appends separately — the two flags are not exclusive.
    #[must_use]
    pub fn with_conversation_config(
        mut self,
        system_prompt: Option<String>,
        append_system_prompt: bool,
        max_turns: Option<u32>,
        disallowed_tools: Vec<String>,
    ) -> Self {
        self.system_prompt = system_prompt;
        self.append_system_prompt = append_system_prompt;
        self.max_turns = max_turns;
        self.disallowed_tools = disallowed_tools;
        self
    }

    #[must_use]
    pub fn with_thinking_enabled(mut self, enabled: Option<bool>) -> Self {
        self.thinking_enabled = enabled;
        self
    }

    /// Appends this immutable turn's documented Claude CLI fields to a launch vector.
    pub fn append_arguments(&self, arguments: &mut Vec<String>) {
        if let Some(enabled) = self.thinking_enabled {
            arguments.extend([
                "--settings".into(),
                serde_json::json!({"alwaysThinkingEnabled": enabled}).to_string(),
            ]);
        }
        arguments.extend(["--model".into(), self.model.clone()]);
        match self.mode {
            ClaudeTurnMode::Chat(permission_mode, effort) => arguments.extend([
                "--effort".into(),
                claude_effort(effort).into(),
                "--permission-prompt-tool".into(),
                "stdio".into(),
                "--permission-mode".into(),
                permission_mode.as_str().into(),
            ]),
            ClaudeTurnMode::Summary => arguments.extend([
                "--safe-mode".into(),
                "--permission-mode".into(),
                "dontAsk".into(),
                "--tools".into(),
                String::new(),
                "--max-turns".into(),
                "1".into(),
                "--no-session-persistence".into(),
            ]),
        }
        let configured_append = self
            .system_prompt
            .as_deref()
            .filter(|_| self.append_system_prompt);
        match (self.instruction, configured_append) {
            (Some(mode_text), Some(configured)) => arguments.extend([
                "--append-system-prompt".into(),
                format!("{mode_text}\n\n{configured}"),
            ]),
            (Some(text), None) | (None, Some(text)) => {
                arguments.extend(["--append-system-prompt".into(), text.into()])
            }
            (None, None) => {}
        }
        if !self.append_system_prompt {
            if let Some(system_prompt) = &self.system_prompt {
                arguments.extend(["--system-prompt".into(), system_prompt.clone()]);
            }
        }
        if let Some(max_turns) = self.max_turns {
            arguments.extend(["--max-turns".into(), max_turns.to_string()]);
        }
        if !self.disallowed_tools.is_empty() {
            arguments.extend(["--disallowedTools".into(), self.disallowed_tools.join(",")]);
        }
    }
}

const fn claude_effort(effort: AgentChatEffort) -> &'static str {
    match effort {
        AgentChatEffort::Low => "low",
        AgentChatEffort::Medium => "medium",
        AgentChatEffort::High => "high",
        AgentChatEffort::XHigh => "xhigh",
        AgentChatEffort::Max | AgentChatEffort::Ultra => "max",
    }
}

fn mode_instruction(mode: AgentChatMode) -> Option<&'static str> {
    match mode {
        AgentChatMode::Ask => Some(
            "Answer and explain. Do not invoke tools or make changes unless the user explicitly asks you to.",
        ),
        AgentChatMode::Plan | AgentChatMode::Agent => None,
    }
}

impl From<PermissionMode> for ClaudePermissionMode {
    fn from(mode: PermissionMode) -> Self {
        match mode {
            PermissionMode::AskEveryTime => Self::Manual,
            PermissionMode::AutoAcceptEdits => Self::AcceptEdits,
            PermissionMode::Autonomous => Self::Auto,
            PermissionMode::Bypass => Self::Bypass,
        }
    }
}

impl ClaudePermissionMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Manual => "manual",
            Self::AcceptEdits => "acceptEdits",
            Self::Auto => "auto",
            Self::Bypass => "bypassPermissions",
        }
    }
}

/// Controlled failure when Gent cannot turn a durable selection into Claude launch fields.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ClaudeTurnOptionsError {
    #[error("the durable selection is not a Claude selection")]
    UnsupportedSelection,
    #[error("the Claude model must be non-empty and within the driver limit")]
    InvalidModel,
}

#[cfg(test)]
#[path = "claude_turn_options_tests.rs"]
mod tests;
