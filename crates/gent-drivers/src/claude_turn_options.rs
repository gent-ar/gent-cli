//! Bounded Claude launch fields derived from a durable Gent run selection.

use gent_types::{AgentChatMode, AgentChatProvider, AgentChatSelection, PermissionMode};

/// Claude launch fields derived only from an immutable Gent run selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaudeTurnOptions {
    model: String,
    mode: ClaudeTurnMode,
    instruction: Option<&'static str>,
    system_prompt: Option<String>,
    append_system_prompt: bool,
    max_turns: Option<u32>,
    disallowed_tools: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClaudeTurnMode {
    Chat(ClaudePermissionMode),
    Summary,
}

/// The complete set of Claude permission modes Gent may request for a chat turn.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaudePermissionMode {
    Default,
    Plan,
    AcceptEdits,
    Auto,
    Bypass,
}

impl ClaudeTurnOptions {
    pub fn from_selection(selection: &AgentChatSelection) -> Result<Self, ClaudeTurnOptionsError> {
        let permission_mode = match selection.mode {
            AgentChatMode::Ask => PermissionMode::Default,
            AgentChatMode::Plan => PermissionMode::Plan,
            AgentChatMode::Agent => PermissionMode::AutoAcceptEdits,
        };
        Self::from_selection_with_permissions(selection, permission_mode)
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
        let permission_mode = effective_permission(selection.mode, permission_mode);
        Ok(Self {
            model: selection.model.clone(),
            mode: ClaudeTurnMode::Chat(ClaudePermissionMode::from(permission_mode)),
            instruction: mode_instruction(selection.mode),
            system_prompt: None,
            append_system_prompt: false,
            max_turns: None,
            disallowed_tools: Vec::new(),
        })
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

    /// Appends this immutable turn's documented Claude CLI fields to a launch vector.
    pub fn append_arguments(&self, arguments: &mut Vec<String>) {
        arguments.extend(["--model".into(), self.model.clone()]);
        match self.mode {
            ClaudeTurnMode::Chat(permission_mode) => {
                arguments.extend(["--permission-mode".into(), permission_mode.as_str().into()])
            }
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

fn effective_permission(mode: AgentChatMode, permission: PermissionMode) -> PermissionMode {
    if mode == AgentChatMode::Plan {
        PermissionMode::Plan
    } else {
        permission
    }
}

fn mode_instruction(mode: AgentChatMode) -> Option<&'static str> {
    match mode {
        AgentChatMode::Ask => Some(
            "Answer and explain. Do not invoke tools or make changes unless the user explicitly asks you to.",
        ),
        AgentChatMode::Plan => Some(
            "You are in Plan Mode. Inspect only as needed, then provide a complete actionable plan. Do not make changes, run write commands, or apply patches; wait for user approval before implementation.",
        ),
        AgentChatMode::Agent => None,
    }
}

impl From<PermissionMode> for ClaudePermissionMode {
    fn from(mode: PermissionMode) -> Self {
        match mode {
            PermissionMode::Default => Self::Default,
            PermissionMode::Plan => Self::Plan,
            PermissionMode::AutoAcceptEdits => Self::AcceptEdits,
            PermissionMode::Autonomous => Self::Auto,
            PermissionMode::Bypass => Self::Bypass,
        }
    }
}

impl ClaudePermissionMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Plan => "plan",
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
mod tests {
    use gent_types::{
        AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, PermissionMode,
    };

    use super::{ClaudeTurnOptions, ClaudeTurnOptionsError};

    #[test]
    fn workspace_permissions_map_to_claude_permission_arguments() {
        for (permission, permission_mode) in [
            (PermissionMode::Default, "default"),
            (PermissionMode::Plan, "plan"),
            (PermissionMode::AutoAcceptEdits, "acceptEdits"),
            (PermissionMode::Autonomous, "auto"),
            (PermissionMode::Bypass, "bypassPermissions"),
        ] {
            let options = ClaudeTurnOptions::from_selection_with_permissions(
                &selection(AgentChatMode::Agent),
                permission,
            )
            .unwrap();
            let mut arguments = Vec::new();
            options.append_arguments(&mut arguments);
            assert_eq!(
                arguments,
                [
                    "--model",
                    "claude-sonnet",
                    "--permission-mode",
                    permission_mode
                ]
            );
        }
    }

    #[test]
    fn chat_mode_instruction_is_independent_from_permission_posture() {
        let mut arguments = Vec::new();
        ClaudeTurnOptions::from_selection_with_permissions(
            &selection(AgentChatMode::Plan),
            PermissionMode::AutoAcceptEdits,
        )
        .unwrap()
        .append_arguments(&mut arguments);
        assert!(
            arguments
                .windows(2)
                .any(|entry| { entry[0] == "--permission-mode" && entry[1] == "plan" })
        );
        assert!(arguments.windows(2).any(|entry| {
            entry[0] == "--append-system-prompt" && entry[1].contains("Plan Mode")
        }));
    }

    #[test]
    fn plan_mode_is_a_non_writing_safety_cap() {
        let mut arguments = Vec::new();
        ClaudeTurnOptions::from_selection_with_permissions(
            &selection(AgentChatMode::Plan),
            PermissionMode::Bypass,
        )
        .unwrap()
        .append_arguments(&mut arguments);
        assert!(
            arguments
                .windows(2)
                .any(|entry| entry[0] == "--permission-mode" && entry[1] == "plan")
        );
    }

    #[test]
    fn non_claude_and_malformed_selections_are_rejected() {
        let mut other = selection(AgentChatMode::Ask);
        other.provider = AgentChatProvider::Codex;
        assert_eq!(
            ClaudeTurnOptions::from_selection_with_permissions(&other, PermissionMode::Default),
            Err(ClaudeTurnOptionsError::UnsupportedSelection)
        );
        let mut malformed = selection(AgentChatMode::Ask);
        malformed.model = "\0".into();
        assert_eq!(
            ClaudeTurnOptions::from_selection_with_permissions(&malformed, PermissionMode::Default),
            Err(ClaudeTurnOptionsError::InvalidModel)
        );
    }

    #[test]
    fn appended_conversation_config_composes_with_the_mode_instruction() {
        let mut arguments = Vec::new();
        ClaudeTurnOptions::from_selection_with_permissions(
            &selection(AgentChatMode::Plan),
            PermissionMode::Default,
        )
        .unwrap()
        .with_conversation_config(Some("Prefer terse replies.".into()), true, None, Vec::new())
        .append_arguments(&mut arguments);
        let append_index = arguments
            .iter()
            .position(|argument| argument == "--append-system-prompt")
            .unwrap();
        let appended = &arguments[append_index + 1];
        assert!(appended.contains("Plan Mode"));
        assert!(appended.contains("Prefer terse replies."));
        assert_eq!(
            arguments
                .iter()
                .filter(|argument| *argument == "--append-system-prompt")
                .count(),
            1
        );
        assert!(!arguments.contains(&"--system-prompt".to_string()));
    }

    #[test]
    fn full_override_replaces_the_default_prompt_but_keeps_the_mode_instruction_appended() {
        let mut arguments = Vec::new();
        ClaudeTurnOptions::from_selection_with_permissions(
            &selection(AgentChatMode::Plan),
            PermissionMode::Default,
        )
        .unwrap()
        .with_conversation_config(
            Some("You are a terse reviewer.".into()),
            false,
            None,
            Vec::new(),
        )
        .append_arguments(&mut arguments);
        assert!(arguments.windows(2).any(|entry| {
            entry[0] == "--append-system-prompt" && entry[1].contains("Plan Mode")
        }));
        assert!(arguments.windows(2).any(|entry| {
            entry[0] == "--system-prompt" && entry[1] == "You are a terse reviewer."
        }));
    }

    #[test]
    fn max_turns_and_disallowed_tools_append_when_configured() {
        let mut arguments = Vec::new();
        ClaudeTurnOptions::from_selection_with_permissions(
            &selection(AgentChatMode::Agent),
            PermissionMode::Default,
        )
        .unwrap()
        .with_conversation_config(
            None,
            false,
            Some(7),
            vec!["shell:rm".into(), "web:fetch".into()],
        )
        .append_arguments(&mut arguments);
        assert!(
            arguments
                .windows(2)
                .any(|entry| entry[0] == "--max-turns" && entry[1] == "7")
        );
        assert!(
            arguments.windows(2).any(|entry| {
                entry[0] == "--disallowedTools" && entry[1] == "shell:rm,web:fetch"
            })
        );
    }

    #[test]
    fn summary_turn_is_fresh_and_has_no_tools_or_persistence() {
        let mut arguments = Vec::new();
        ClaudeTurnOptions::summary("haiku")
            .unwrap()
            .append_arguments(&mut arguments);
        assert_eq!(
            arguments,
            [
                "--model",
                "haiku",
                "--safe-mode",
                "--permission-mode",
                "dontAsk",
                "--tools",
                "",
                "--max-turns",
                "1",
                "--no-session-persistence",
            ]
        );
    }

    fn selection(mode: AgentChatMode) -> AgentChatSelection {
        AgentChatSelection {
            provider: AgentChatProvider::Claude,
            model: "claude-sonnet".into(),
            effort: AgentChatEffort::High,
            mode,
        }
    }
}
