use crate::local_ipc::connect_and_negotiate;
use clap::Subcommand;
use gent_protocol::{WORKSPACE_DOCUMENTS_CAPABILITY, WorkspaceDocumentsFrame};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Subcommand)]
pub(crate) enum WorkspaceDocumentsCommand {
    List {
        #[arg(long, default_value = "workspace-1")]
        workspace_id: String,
    },
}

pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    command: WorkspaceDocumentsCommand,
) -> Result<Value, Box<dyn std::error::Error>> {
    let WorkspaceDocumentsCommand::List { workspace_id } = command;
    let frame = WorkspaceDocumentsFrame::List {
        request_id: uuid::Uuid::new_v4().to_string(),
        workspace_id,
    };
    frame.validate()?;
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|value| value == WORKSPACE_DOCUMENTS_CAPABILITY)
    {
        return Err("workspace documents capability is unavailable".into());
    }
    gent_protocol::write_json_frame(&mut stream, &frame).await?;
    let reply: WorkspaceDocumentsFrame = gent_protocol::read_json_frame(&mut stream).await?;
    Ok(serde_json::to_value(reply)?)
}

pub(crate) async fn list(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    workspace_id: String,
) -> Result<Vec<gent_protocol::WorkspaceDocumentRecord>, Box<dyn std::error::Error>> {
    let value = execute(
        data_dir,
        no_autostart,
        WorkspaceDocumentsCommand::List { workspace_id },
    )
    .await?;
    let frame: WorkspaceDocumentsFrame = serde_json::from_value(value)?;
    match frame {
        WorkspaceDocumentsFrame::Listed { documents, .. } => Ok(documents),
        _ => Err("invalid workspace document response".into()),
    }
}
