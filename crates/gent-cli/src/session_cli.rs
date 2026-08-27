use std::path::PathBuf;

use gent_protocol::{
    AGENT_CHAT_SESSIONS_CAPABILITY, AgentChatSessionFrame, read_json_frame, write_json_frame,
};
use gent_types::{AgentChatSession, AgentChatSessionId};

use crate::local_ipc::connect_and_negotiate;

pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    action: crate::SessionCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    let request_id = uuid::Uuid::new_v4().to_string();
    let frame = match action {
        crate::SessionCommand::List { workspace_id } => AgentChatSessionFrame::ListRequest {
            request_id,
            workspace_id,
        },
        crate::SessionCommand::Create { session } => AgentChatSessionFrame::CreateRequest {
            request_id,
            session: serde_json::from_str::<AgentChatSession>(&session)?,
        },
        crate::SessionCommand::Select { session_id } => AgentChatSessionFrame::SelectRequest {
            request_id,
            session_id: AgentChatSessionId(session_id),
        },
        crate::SessionCommand::Attach {
            session_id,
            conversation_id,
        } => AgentChatSessionFrame::AttachRequest {
            request_id,
            session_id: AgentChatSessionId(session_id),
            conversation_id,
        },
    };
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|item| item == AGENT_CHAT_SESSIONS_CAPABILITY)
    {
        return Err("gentd does not advertise agent chat sessions".into());
    }
    write_json_frame(&mut stream, &frame).await?;
    println!(
        "{}",
        serde_json::to_string_pretty::<serde_json::Value>(&read_json_frame(&mut stream).await?)?
    );
    Ok(())
}

pub(crate) async fn list(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    workspace_id: String,
) -> Result<Vec<AgentChatSession>, Box<dyn std::error::Error>> {
    let response = exchange(
        data_dir,
        no_autostart,
        AgentChatSessionFrame::ListRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            workspace_id,
        },
    )
    .await?;
    match response {
        AgentChatSessionFrame::List { sessions, .. } => Ok(sessions),
        _ => Err("gentd returned an invalid session list".into()),
    }
}

pub(crate) async fn create(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    session: AgentChatSession,
) -> Result<AgentChatSession, Box<dyn std::error::Error>> {
    let response = exchange(
        data_dir,
        no_autostart,
        AgentChatSessionFrame::CreateRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            session,
        },
    )
    .await?;
    match response {
        AgentChatSessionFrame::Created { session, .. } => Ok(session),
        _ => Err("gentd returned an invalid session create response".into()),
    }
}

pub(crate) async fn attach(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    session_id: AgentChatSessionId,
    conversation_id: String,
) -> Result<AgentChatSession, Box<dyn std::error::Error>> {
    let response = exchange(
        data_dir,
        no_autostart,
        AgentChatSessionFrame::AttachRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            session_id,
            conversation_id,
        },
    )
    .await?;
    match response {
        AgentChatSessionFrame::Attached { session, .. } => Ok(session),
        _ => Err("gentd returned an invalid session attach response".into()),
    }
}

async fn exchange(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    frame: AgentChatSessionFrame,
) -> Result<AgentChatSessionFrame, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|item| item == AGENT_CHAT_SESSIONS_CAPABILITY)
    {
        return Err("gentd does not advertise agent chat sessions".into());
    }
    write_json_frame(&mut stream, &frame).await?;
    Ok(read_json_frame(&mut stream).await?)
}
