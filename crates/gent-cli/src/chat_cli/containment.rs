use std::path::PathBuf;

use gent_protocol::{
    AGENT_CHAT_CONVERSATION_CONFIG_CAPABILITY, AgentChatConversationConfigFrame,
    AgentChatIntentFrame, read_json_frame, write_json_frame,
};
use gent_types::{AgentChatConversationConfigRecord, AgentChatConversationId, Containment};
use serde_json::Value;

use super::{CreateArgs, exchange, new_selection, receipt_id, request_id, workspace_path};
use crate::local_ipc::connect_and_negotiate;

pub(super) async fn create(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    args: CreateArgs,
) -> Result<AgentChatIntentFrame, Box<dyn std::error::Error>> {
    let sandboxed = args.sandboxed;
    let request = AgentChatIntentFrame::CreateConversation {
        request_id: request_id(args.request_id),
        receipt_id: receipt_id(args.receipt_id),
        workspace_path: workspace_path(args.workspace)?,
        selection: new_selection(data_dir.clone(), no_autostart, args.selection.request()).await?,
    };
    let created = exchange(data_dir.clone(), no_autostart, request).await?;
    if sandboxed {
        if let AgentChatIntentFrame::Created {
            conversation_id, ..
        } = &created
        {
            save(
                data_dir,
                no_autostart,
                conversation_id,
                Containment::Sandboxed,
            )
            .await?;
        }
    }
    Ok(created)
}

pub(crate) async fn save(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: &AgentChatConversationId,
    containment: Containment,
) -> Result<AgentChatConversationConfigRecord, Box<dyn std::error::Error>> {
    let AgentChatConversationConfigFrame::CurrentConfig {
        config: current, ..
    } = config_exchange(
        data_dir.clone(),
        no_autostart,
        AgentChatConversationConfigFrame::Current {
            request_id: request_id(None).0,
            conversation_id: conversation_id.0.clone(),
        },
    )
    .await?
    else {
        return Err("daemon did not return the conversation config".into());
    };
    let config = AgentChatConversationConfigRecord {
        revision: current.as_ref().map_or(1, |config| config.revision + 1),
        containment,
        ..current.unwrap_or(AgentChatConversationConfigRecord {
            conversation_id: conversation_id.clone(),
            revision: 1,
            system_prompt: None,
            append_system_prompt: false,
            max_turns: None,
            thinking_enabled: None,
            disallowed_tools: Vec::new(),
            containment,
        })
    };
    match config_exchange(
        data_dir,
        no_autostart,
        AgentChatConversationConfigFrame::Save {
            request_id: request_id(None).0,
            config,
        },
    )
    .await?
    {
        AgentChatConversationConfigFrame::Saved { config, .. } => Ok(config),
        _ => Err("daemon did not confirm the conversation config".into()),
    }
}

async fn config_exchange(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    request: AgentChatConversationConfigFrame,
) -> Result<AgentChatConversationConfigFrame, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|item| item == AGENT_CHAT_CONVERSATION_CONFIG_CAPABILITY)
    {
        return Err("daemon does not support conversation config; upgrade gentd".into());
    }
    write_json_frame(&mut stream, &request).await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Some(error) = crate::cli_error::CliError::from_reply(&raw) {
        return Err(error.into());
    }
    Ok(serde_json::from_value(raw).map_err(|_| "daemon did not return a conversation config")?)
}
