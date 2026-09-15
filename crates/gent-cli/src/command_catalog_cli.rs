use std::path::PathBuf;

use gent_protocol::{
    agent_chat_commands::{
        AGENT_CHAT_COMMANDS_CAPABILITY, AgentChatCommandFrame, CommandCatalog, CommandOutcome,
    },
    read_json_frame, write_json_frame,
};
use gent_types::{AgentChatCommandDescriptor, AgentChatConversationId, AgentChatRequestId};
use serde_json::Value;

use crate::local_ipc::connect_and_negotiate;

pub(crate) async fn read(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: Option<String>,
) -> Result<Option<CommandCatalog>, Box<dyn std::error::Error>> {
    let workspace_path = workspace_path(conversation_id.is_none())?;
    let reply = exchange(data_dir, no_autostart, |request_id| {
        AgentChatCommandFrame::ReadCommandCatalog {
            request_id,
            conversation_id: conversation_id.map(AgentChatConversationId),
            workspace_path,
            refresh: false,
        }
    })
    .await?;
    match reply {
        Some(AgentChatCommandFrame::CommandCatalog { catalog, .. }) => Ok(Some(catalog)),
        Some(_) => Err("daemon did not return a command catalog".into()),
        None => Ok(None),
    }
}

pub(crate) async fn invoke(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: Option<String>,
    receipt_id: Option<String>,
    name: String,
    arguments: String,
) -> Result<(gent_types::Receipt, CommandOutcome), Box<dyn std::error::Error>> {
    let workspace_path = workspace_path(conversation_id.is_none())?;
    let reply = exchange(data_dir, no_autostart, |request_id| {
        AgentChatCommandFrame::InvokeCommand {
            request_id,
            receipt_id: receipt_id.map_or_else(gent_types::ReceiptId::new, gent_types::ReceiptId),
            workspace_path,
            conversation_id: conversation_id.map(AgentChatConversationId),
            name,
            arguments,
        }
    })
    .await?;
    match reply {
        Some(AgentChatCommandFrame::CommandInvoked {
            receipt, outcome, ..
        }) => Ok((receipt, outcome)),
        Some(_) => Err("daemon did not return a command outcome".into()),
        None => Err("gentd does not expose agent-chat commands; upgrade gentd".into()),
    }
}

pub(crate) fn resolve<'a>(
    catalog: &'a CommandCatalog,
    name: &str,
) -> Option<&'a AgentChatCommandDescriptor> {
    catalog
        .commands
        .iter()
        .find(|command| command.answers_to(name))
}

fn workspace_path(
    without_conversation: bool,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    without_conversation
        .then(|| crate::chat_cli::workspace_path(None))
        .transpose()
}

async fn exchange(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    request: impl FnOnce(AgentChatRequestId) -> AgentChatCommandFrame,
) -> Result<Option<AgentChatCommandFrame>, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|capability| capability == AGENT_CHAT_COMMANDS_CAPABILITY)
    {
        return Ok(None);
    }
    let request_id = AgentChatRequestId(uuid::Uuid::new_v4().to_string());
    let frame = request(request_id.clone());
    frame.validate()?;
    write_json_frame(&mut stream, &frame).await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Some(error) = crate::cli_error::CliError::from_reply(&raw) {
        return Err(error.into());
    }
    let reply: AgentChatCommandFrame = serde_json::from_value(raw)
        .map_err(|_| "daemon did not return an agent-chat command reply")?;
    let correlated = match &reply {
        AgentChatCommandFrame::CommandCatalog {
            request_id: reply_id,
            ..
        }
        | AgentChatCommandFrame::CommandInvoked {
            request_id: reply_id,
            ..
        } => *reply_id == request_id,
        _ => false,
    };
    correlated
        .then_some(Some(reply))
        .ok_or_else(|| "daemon returned a command reply for a different request".into())
}
