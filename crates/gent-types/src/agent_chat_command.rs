use serde::{Deserialize, Serialize};

use crate::AgentChatProvider;

pub const MAX_COMMAND_NAME_BYTES: usize = 64;
pub const MAX_COMMAND_TEXT_BYTES: usize = 512;
pub const MAX_COMMAND_ARGUMENTS_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AgentChatCommandOrigin {
    Gent,
    ProviderBuiltin { provider: AgentChatProvider },
    ProviderSkill { provider: AgentChatProvider },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum AgentChatCommandIntent {
    CreateConversation,
    ClearContext,
    ForkConversation,
    SelectProvider,
    SelectModel,
    SelectEffort,
    SelectMode,
    Goal,
    Compact,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum AgentChatClientAction {
    ConversationPicker,
    Help,
    ProviderLogin,
    Rename,
    SideQuestion,
    Attach,
    Detach,
    Search,
    Thinking,
    Activity,
    SteerQueued,
    CancelQueued,
    ContinueFromHistory,
    Decision,
    PermissionSettings,
    Automations,
    Sessions,
    Tools,
    Git,
    Templates,
    Documents,
    ApplySelection,
    ContextPolicy,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AgentChatCommandDispatch {
    GentIntent {
        intent: AgentChatCommandIntent,
    },
    ClientAction {
        action: AgentChatClientAction,
    },
    ProviderNative,
    Unsupported {
        reason: String,
        use_instead: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentChatCommandAvailability {
    pub requires_conversation: bool,
    pub blocked_while_turn_active: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentChatCommandDescriptor {
    pub name: String,
    pub aliases: Vec<String>,
    pub description: String,
    pub argument_hint: Option<String>,
    pub origin: AgentChatCommandOrigin,
    pub dispatch: AgentChatCommandDispatch,
    pub availability: AgentChatCommandAvailability,
}

impl AgentChatCommandDescriptor {
    #[must_use]
    pub fn answers_to(&self, name: &str) -> bool {
        self.name == name || self.aliases.iter().any(|alias| alias == name)
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        valid_command_name(&self.name)
            && self.aliases.iter().all(|alias| valid_command_name(alias))
            && valid_command_text(&self.description)
            && self.argument_hint.as_deref().is_none_or(valid_command_text)
            && match &self.dispatch {
                AgentChatCommandDispatch::Unsupported {
                    reason,
                    use_instead,
                } => {
                    valid_command_text(reason)
                        && use_instead.as_deref().is_none_or(valid_command_text)
                }
                _ => true,
            }
    }
}

#[must_use]
pub fn valid_command_name(name: &str) -> bool {
    let mut characters = name.chars();
    name.len() <= MAX_COMMAND_NAME_BYTES
        && characters
            .next()
            .is_some_and(|first| first.is_ascii_alphanumeric())
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ':' | '_' | '-')
        })
}

fn valid_command_text(text: &str) -> bool {
    !text.trim().is_empty()
        && text.len() <= MAX_COMMAND_TEXT_BYTES
        && !text.contains(char::is_control)
}

#[must_use]
pub fn slash_command(text: &str) -> Option<(&str, &str)> {
    let rest = text.trim_start().strip_prefix('/')?;
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let name = &rest[..end];
    valid_command_name(name).then(|| (name, rest[end..].trim()))
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AgentChatCommandRejection {
    #[error("/{name} is not a command in this conversation's catalog")]
    UnknownCommand { name: String },
    #[error("/{name} is not supported by Gent: {reason}{}", use_instead.as_ref().map(|value| format!(" Use {value} instead.")).unwrap_or_default())]
    UnsupportedCommand {
        name: String,
        reason: String,
        use_instead: Option<String>,
    },
    #[error("/{name} is handled by the client, not by Gentd")]
    ClientActionCommand { name: String },
    #[error("/{name} needs a conversation")]
    CommandRequiresConversation { name: String },
    #[error("/{name} waits until the current turn settles")]
    CommandBlockedByActiveTurn { name: String },
    #[error("/{name} needs {hint}")]
    CommandArgumentsInvalid { name: String, hint: String },
    #[error("/{name} needs a provider session; send a message in this run first")]
    CommandRequiresProviderSession { name: String },
    #[error("the provider's commands are still loading; try /{name} again shortly")]
    CommandCatalogLoading { name: String },
    #[error(
        "/{name} is a command; invoke it through the command catalog instead of sending it as a prompt"
    )]
    SlashCommandRequiresInvoke { name: String },
}

impl AgentChatCommandRejection {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnknownCommand { .. } => "unknownCommand",
            Self::UnsupportedCommand { .. } => "unsupportedCommand",
            Self::ClientActionCommand { .. } => "clientActionCommand",
            Self::CommandRequiresConversation { .. } => "commandRequiresConversation",
            Self::CommandBlockedByActiveTurn { .. } => "commandBlockedByActiveTurn",
            Self::CommandArgumentsInvalid { .. } => "commandArgumentsInvalid",
            Self::CommandRequiresProviderSession { .. } => "commandRequiresProviderSession",
            Self::CommandCatalogLoading { .. } => "commandCatalogLoading",
            Self::SlashCommandRequiresInvoke { .. } => "slashCommandRequiresInvoke",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentChatCommandRejection, slash_command, valid_command_name};

    #[test]
    fn only_a_leading_command_token_is_command_shaped() {
        assert_eq!(
            slash_command("/compact keep tests"),
            Some(("compact", "keep tests"))
        );
        assert_eq!(slash_command("  /context"), Some(("context", "")));
        assert_eq!(
            slash_command("/plugin:skill\targ"),
            Some(("plugin:skill", "arg"))
        );
        assert_eq!(slash_command("/Users/x/file.rs explain"), None);
        assert_eq!(slash_command("/ spaced"), None);
        assert_eq!(slash_command("-/help"), None);
        assert_eq!(slash_command("explain /help"), None);
        assert!(!valid_command_name(&"a".repeat(65)));
    }

    #[test]
    fn unsupported_rejection_names_the_replacement() {
        let rejection = AgentChatCommandRejection::UnsupportedCommand {
            name: "config".into(),
            reason: "provider settings diverge from Gent's.".into(),
            use_instead: Some("/permissions".into()),
        };
        assert_eq!(rejection.code(), "unsupportedCommand");
        assert_eq!(
            rejection.to_string(),
            "/config is not supported by Gent: provider settings diverge from Gent's. Use /permissions instead."
        );
    }
}
