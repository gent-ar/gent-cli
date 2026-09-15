use gent_types::{
    AgentChatCommandDescriptor, AgentChatCommandIntent, AgentChatConversationId,
    AgentChatPromptDelivery, AgentChatProvider, AgentChatRequestId, AgentChatRunId,
    MAX_COMMAND_ARGUMENTS_BYTES, Receipt, ReceiptId, valid_command_name,
};
use serde::{Deserialize, Serialize};

pub const AGENT_CHAT_COMMANDS_CAPABILITY: &str = "agent-chat-commands-v1";

const MAX_COMMANDS: usize = 512;
const MAX_IDENTIFIER_BYTES: usize = 128;
const MAX_PATH_BYTES: usize = 4096;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AgentChatCommandFrame {
    ReadCommandCatalog {
        request_id: AgentChatRequestId,
        conversation_id: Option<AgentChatConversationId>,
        workspace_path: Option<String>,
        refresh: bool,
    },
    CommandCatalog {
        request_id: AgentChatRequestId,
        catalog: CommandCatalog,
    },
    InvokeCommand {
        request_id: AgentChatRequestId,
        receipt_id: ReceiptId,
        conversation_id: Option<AgentChatConversationId>,
        workspace_path: Option<String>,
        name: String,
        arguments: String,
    },
    CommandInvoked {
        request_id: AgentChatRequestId,
        receipt: Receipt,
        conversation_id: Option<AgentChatConversationId>,
        outcome: CommandOutcome,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandCatalog {
    pub scope: CommandCatalogScope,
    pub revision: String,
    pub provider_version: Option<String>,
    pub listing: CommandListing,
    pub commands: Vec<AgentChatCommandDescriptor>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandCatalogScope {
    pub conversation_id: Option<AgentChatConversationId>,
    pub workspace_path: Option<String>,
    pub provider: Option<AgentChatProvider>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CommandListing {
    Loading,
    Ready,
    Failed { message: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CommandOutcome {
    Delivered {
        run_id: AgentChatRunId,
        turn_id: String,
        message_id: String,
        delivery: AgentChatPromptDelivery,
    },
    IntentApplied {
        intent: AgentChatCommandIntent,
        conversation_id: AgentChatConversationId,
        run_id: Option<AgentChatRunId>,
    },
}

#[derive(Clone, Copy, Debug, thiserror::Error, Eq, PartialEq)]
pub enum AgentChatCommandFrameError {
    #[error("invalid command request identifier")]
    InvalidIdentifier,
    #[error("invalid command scope")]
    InvalidScope,
    #[error("invalid command name or arguments")]
    InvalidInvocation,
    #[error("invalid command catalog")]
    InvalidCatalog,
}

impl AgentChatCommandFrame {
    #[must_use]
    pub const fn is_client_request(&self) -> bool {
        matches!(
            self,
            Self::ReadCommandCatalog { .. } | Self::InvokeCommand { .. }
        )
    }

    pub fn validate(&self) -> Result<(), AgentChatCommandFrameError> {
        match self {
            Self::ReadCommandCatalog {
                request_id,
                conversation_id,
                workspace_path,
                ..
            } => {
                identifier(&request_id.0)?;
                scope(conversation_id.as_ref(), workspace_path.as_deref())
            }
            Self::InvokeCommand {
                request_id,
                receipt_id,
                conversation_id,
                workspace_path,
                name,
                arguments,
            } => {
                identifier(&request_id.0)?;
                identifier(&receipt_id.0)?;
                scope(conversation_id.as_ref(), workspace_path.as_deref())?;
                (valid_command_name(name)
                    && arguments.len() <= MAX_COMMAND_ARGUMENTS_BYTES
                    && !arguments.contains('\0'))
                .then_some(())
                .ok_or(AgentChatCommandFrameError::InvalidInvocation)
            }
            Self::CommandCatalog {
                request_id,
                catalog,
            } => {
                identifier(&request_id.0)?;
                catalog.validate()
            }
            Self::CommandInvoked {
                request_id,
                conversation_id,
                ..
            } => {
                identifier(&request_id.0)?;
                scope(conversation_id.as_ref(), None)
            }
        }
    }
}

impl CommandCatalog {
    fn validate(&self) -> Result<(), AgentChatCommandFrameError> {
        scope(
            self.scope.conversation_id.as_ref(),
            self.scope.workspace_path.as_deref(),
        )?;
        let mut names = std::collections::BTreeSet::new();
        let valid = identifier(&self.revision).is_ok()
            && self
                .provider_version
                .as_deref()
                .is_none_or(|version| identifier(version).is_ok())
            && !matches!(&self.listing, CommandListing::Failed { message } if message.trim().is_empty() || message.len() > 512)
            && self.commands.len() <= MAX_COMMANDS
            && self.commands.iter().all(|command| {
                command.is_valid()
                    && std::iter::once(&command.name)
                        .chain(&command.aliases)
                        .all(|name| names.insert(name.as_str()))
            });
        valid
            .then_some(())
            .ok_or(AgentChatCommandFrameError::InvalidCatalog)
    }
}

fn identifier(value: &str) -> Result<(), AgentChatCommandFrameError> {
    (!value.trim().is_empty()
        && value.trim() == value
        && value.len() <= MAX_IDENTIFIER_BYTES
        && !value.contains(char::is_control))
    .then_some(())
    .ok_or(AgentChatCommandFrameError::InvalidIdentifier)
}

fn scope(
    conversation_id: Option<&AgentChatConversationId>,
    workspace_path: Option<&str>,
) -> Result<(), AgentChatCommandFrameError> {
    let valid_conversation = conversation_id.is_none_or(|id| identifier(&id.0).is_ok());
    let valid_path = workspace_path.is_none_or(|path| {
        !path.trim().is_empty() && path.len() <= MAX_PATH_BYTES && !path.contains('\0')
    });
    (valid_conversation && valid_path)
        .then_some(())
        .ok_or(AgentChatCommandFrameError::InvalidScope)
}

#[cfg(test)]
#[path = "agent_chat_commands_tests.rs"]
mod tests;
