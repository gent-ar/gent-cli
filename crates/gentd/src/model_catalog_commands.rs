use std::{path::PathBuf, sync::Arc};

use gent_protocol::agent_chat_commands::CommandListing;
use gent_types::{AgentChatCommandDescriptor, AgentChatProvider};

use super::claude_initialize::{ClaudeInitialize, InitializeState};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProviderCommands {
    pub(crate) listing: CommandListing,
    pub(crate) commands: Vec<AgentChatCommandDescriptor>,
}

impl ProviderCommands {
    pub(crate) const fn none() -> Self {
        Self {
            listing: CommandListing::Ready,
            commands: Vec::new(),
        }
    }

    fn failed(message: impl Into<String>) -> Self {
        Self {
            listing: CommandListing::Failed {
                message: message.into(),
            },
            commands: Vec::new(),
        }
    }
}

pub(crate) fn provider_commands(
    initialize: &Arc<ClaudeInitialize>,
    provider: AgentChatProvider,
    workspace: Option<PathBuf>,
    refresh: bool,
) -> ProviderCommands {
    if provider != AgentChatProvider::Claude {
        return ProviderCommands::none();
    }
    match initialize.state(workspace, refresh) {
        InitializeState::NotInstalled => ProviderCommands::failed("Claude is not installed"),
        InitializeState::Loading => ProviderCommands {
            listing: CommandListing::Loading,
            commands: Vec::new(),
        },
        InitializeState::Failed(message) => ProviderCommands::failed(message),
        InitializeState::Listed(response) => {
            gent_drivers::claude_commands::claude_command_descriptors(&response).map_or_else(
                || ProviderCommands::failed("claude did not report its commands"),
                |commands| ProviderCommands {
                    listing: CommandListing::Ready,
                    commands,
                },
            )
        }
    }
}
