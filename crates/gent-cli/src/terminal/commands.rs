use gent_protocol::agent_chat_commands::CommandCatalog;
use gent_types::{
    AgentChatClientAction as Action, AgentChatCommandDescriptor, AgentChatCommandDispatch,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct CommandState {
    pub(crate) home: Option<CommandCatalog>,
    pub(crate) picker: Option<usize>,
}

pub(crate) const fn implemented(action: Action) -> bool {
    !matches!(action, Action::Rename | Action::SideQuestion)
}

pub(crate) fn visible(command: &AgentChatCommandDescriptor) -> bool {
    match command.dispatch {
        AgentChatCommandDispatch::ClientAction { action } => implemented(action),
        _ => true,
    }
}

pub(crate) fn palette<'a>(
    catalog: &'a CommandCatalog,
    input: &str,
) -> Vec<&'a AgentChatCommandDescriptor> {
    let Some(query) = input.strip_prefix('/') else {
        return Vec::new();
    };
    if query.contains(char::is_whitespace) {
        return Vec::new();
    }
    catalog
        .commands
        .iter()
        .filter(|command| {
            visible(command)
                && std::iter::once(&command.name)
                    .chain(&command.aliases)
                    .any(|name| name.starts_with(query))
        })
        .collect()
}

pub(crate) fn usage(command: &AgentChatCommandDescriptor) -> String {
    let hint = command
        .argument_hint
        .as_ref()
        .map(|hint| format!(" {hint}"))
        .unwrap_or_default();
    let note = match &command.dispatch {
        AgentChatCommandDispatch::Unsupported { .. } => " · unsupported",
        _ => "",
    };
    format!("/{}{hint} — {}{note}", command.name, command.description)
}

#[cfg(test)]
pub(crate) mod tests {
    use gent_protocol::agent_chat_commands::{CommandCatalog, CommandCatalogScope, CommandListing};

    use super::{palette, usage};
    use crate::terminal::UiState;

    pub(crate) fn catalog() -> CommandCatalog {
        catalog_for(None)
    }

    pub(crate) fn catalog_for(provider: Option<gent_types::AgentChatProvider>) -> CommandCatalog {
        CommandCatalog {
            scope: CommandCatalogScope {
                conversation_id: None,
                workspace_path: None,
                provider,
            },
            revision: "sha256:test".into(),
            provider_version: None,
            listing: CommandListing::Ready,
            commands: gent_core::command_catalog(provider, []),
        }
    }

    pub(crate) fn catalog_with_unsupported() -> CommandCatalog {
        let mut catalog = catalog();
        catalog
            .commands
            .push(gent_types::AgentChatCommandDescriptor {
                name: "mcp".into(),
                aliases: Vec::new(),
                description: "Configure MCP servers".into(),
                argument_hint: None,
                origin: gent_types::AgentChatCommandOrigin::ProviderBuiltin {
                    provider: gent_types::AgentChatProvider::Claude,
                },
                dispatch: gent_types::AgentChatCommandDispatch::Unsupported {
                    reason: "MCP servers are configured through Gent.".into(),
                    use_instead: Some("/tools".into()),
                },
                availability: gent_types::AgentChatCommandAvailability {
                    requires_conversation: true,
                    blocked_while_turn_active: true,
                },
            });
        catalog
    }

    #[test]
    fn the_palette_lists_catalog_commands_the_terminal_implements() {
        let catalog = catalog();
        let names = |input| {
            palette(&catalog, input)
                .into_iter()
                .map(|command| command.name.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(names("/re"), ["clear", "resume"]);
        assert!(names("/b").is_empty());
        assert_eq!(names("/automation"), ["automations"]);
        assert!(names("/compact now").is_empty());
        let local = catalog_for(Some(gent_types::AgentChatProvider::Claurst));
        let compact = local
            .commands
            .iter()
            .find(|command| command.name == "compact")
            .unwrap();
        assert_eq!(
            usage(compact),
            "/compact — Compact the conversation context"
        );
        assert_eq!(
            palette(&local, "/comp")
                .into_iter()
                .map(|command| command.name.as_str())
                .collect::<Vec<_>>(),
            ["compact"]
        );
        let unsupported = catalog_with_unsupported();
        let mcp = unsupported
            .commands
            .iter()
            .find(|command| command.name == "mcp")
            .unwrap();
        assert!(usage(mcp).ends_with("· unsupported"));
        let state = UiState::new(Vec::new()).with_command_catalog(Some(catalog.clone()));
        assert_eq!(state.command_catalog(), Some(&catalog));
    }
}
