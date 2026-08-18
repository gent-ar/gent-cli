//! Bounded Claude launch fields derived from a durable Gent run selection.

use gent_types::{AgentChatMode, AgentChatProvider, AgentChatSelection};

/// Claude launch fields derived only from an immutable Gent run selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaudeTurnOptions {
    model: String,
    permission_mode: ClaudePermissionMode,
}

/// The complete set of Claude permission modes Gent may request for a chat turn.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaudePermissionMode {
    Default,
    Plan,
    AcceptEdits,
}

impl ClaudeTurnOptions {
    /// Converts a durable Claude selection into the exact documented CLI arguments.
    ///
    /// Gent deliberately never selects Claude's autonomous or bypass-permission modes.
    ///
    /// # Errors
    /// Returns an error for another provider or an invalid model identifier.
    pub fn from_selection(selection: &AgentChatSelection) -> Result<Self, ClaudeTurnOptionsError> {
        if selection.provider != AgentChatProvider::Claude {
            return Err(ClaudeTurnOptionsError::UnsupportedSelection);
        }
        if selection.validate().is_err() {
            return Err(ClaudeTurnOptionsError::InvalidModel);
        }
        let permission_mode = match selection.mode {
            AgentChatMode::Ask => ClaudePermissionMode::Default,
            AgentChatMode::Plan => ClaudePermissionMode::Plan,
            AgentChatMode::Agent => ClaudePermissionMode::AcceptEdits,
        };
        Ok(Self {
            model: selection.model.clone(),
            permission_mode,
        })
    }

    /// Appends this immutable turn's documented Claude CLI fields to a launch vector.
    pub(crate) fn append_arguments(&self, arguments: &mut Vec<String>) {
        arguments.extend([
            "--model".into(),
            self.model.clone(),
            "--permission-mode".into(),
            self.permission_mode.as_str().into(),
        ]);
    }
}

impl ClaudePermissionMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Plan => "plan",
            Self::AcceptEdits => "acceptEdits",
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
    use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection};

    use super::{ClaudeTurnOptions, ClaudeTurnOptionsError};

    #[test]
    fn durable_modes_map_only_to_bounded_claude_permission_arguments() {
        for (mode, permission_mode) in [
            (AgentChatMode::Ask, "default"),
            (AgentChatMode::Plan, "plan"),
            (AgentChatMode::Agent, "acceptEdits"),
        ] {
            let options = ClaudeTurnOptions::from_selection(&selection(mode)).unwrap();
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
            assert!(
                !arguments
                    .iter()
                    .any(|value| value == "auto" || value == "bypassPermissions")
            );
        }
    }

    #[test]
    fn non_claude_and_malformed_selections_are_rejected() {
        let mut other = selection(AgentChatMode::Ask);
        other.provider = AgentChatProvider::Codex;
        assert_eq!(
            ClaudeTurnOptions::from_selection(&other),
            Err(ClaudeTurnOptionsError::UnsupportedSelection)
        );
        let mut malformed = selection(AgentChatMode::Ask);
        malformed.model = "\0".into();
        assert_eq!(
            ClaudeTurnOptions::from_selection(&malformed),
            Err(ClaudeTurnOptionsError::InvalidModel)
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
