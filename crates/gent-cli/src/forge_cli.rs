use std::path::PathBuf;

use gent_protocol::{
    FORGE_CONNECTORS_CAPABILITY, ForgeConnectorFrame, WireFrame, read_json_frame, write_json_frame,
};
use gent_types::ForgeConnectorRecord;
use serde_json::Value;

use crate::ForgeCommand;
use crate::local_ipc::connect_and_negotiate;

pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    action: ForgeCommand,
) -> Result<Value, Box<dyn std::error::Error>> {
    let frame = match action {
        ForgeCommand::List { workspace_id } => ForgeConnectorFrame::ListRequest {
            request_id: id(),
            workspace_id,
        },
        ForgeCommand::Get {
            workspace_id,
            connector_id,
        } => ForgeConnectorFrame::GetRequest {
            request_id: id(),
            workspace_id,
            connector_id,
        },
        ForgeCommand::Create { connector } => ForgeConnectorFrame::CreateRequest {
            request_id: id(),
            connector: serde_json::from_str(&connector)?,
        },
        ForgeCommand::Enable {
            workspace_id,
            connector_id,
        } => ForgeConnectorFrame::SetEnabledRequest {
            request_id: id(),
            workspace_id,
            connector_id,
            enabled: true,
        },
        ForgeCommand::Disable {
            workspace_id,
            connector_id,
        } => ForgeConnectorFrame::SetEnabledRequest {
            request_id: id(),
            workspace_id,
            connector_id,
            enabled: false,
        },
        ForgeCommand::Invoke {
            workspace_id,
            connector_id,
            tool_name,
        } => ForgeConnectorFrame::InvokeRequest {
            request_id: id(),
            workspace_id,
            connector_id,
            tool_name,
        },
    };
    Ok(serde_json::to_value(
        exchange(data_dir, no_autostart, frame).await?,
    )?)
}

pub(crate) async fn list(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    workspace_id: String,
) -> Result<Vec<ForgeConnectorRecord>, Box<dyn std::error::Error>> {
    let frame = exchange(
        data_dir,
        no_autostart,
        ForgeConnectorFrame::ListRequest {
            request_id: id(),
            workspace_id: workspace_id.clone(),
        },
    )
    .await?;
    match frame {
        ForgeConnectorFrame::List {
            connectors,
            workspace_id: response_workspace,
            ..
        } if response_workspace == workspace_id => Ok(connectors),
        _ => Err("daemon returned an invalid Forge catalog".into()),
    }
}

async fn exchange(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    frame: ForgeConnectorFrame,
) -> Result<ForgeConnectorFrame, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|capability| capability == FORGE_CONNECTORS_CAPABILITY)
    {
        return Err("daemon does not support Gent Forge catalogs; upgrade gentd".into());
    }
    let expected_id = request_id(&frame);
    write_json_frame(&mut stream, &frame).await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw.clone()) {
        return Err(message.into());
    }
    let reply: ForgeConnectorFrame = serde_json::from_value(raw)?;
    if request_id(&reply) != expected_id {
        return Err("daemon returned an uncorrelated Forge response".into());
    }
    reply.validate().map_err(|error| error.to_string())?;
    Ok(reply)
}

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn request_id(frame: &ForgeConnectorFrame) -> String {
    match frame {
        ForgeConnectorFrame::ListRequest { request_id, .. }
        | ForgeConnectorFrame::List { request_id, .. }
        | ForgeConnectorFrame::GetRequest { request_id, .. }
        | ForgeConnectorFrame::Get { request_id, .. }
        | ForgeConnectorFrame::CreateRequest { request_id, .. }
        | ForgeConnectorFrame::Created { request_id, .. }
        | ForgeConnectorFrame::SetEnabledRequest { request_id, .. }
        | ForgeConnectorFrame::SetEnabled { request_id, .. }
        | ForgeConnectorFrame::InvokeRequest { request_id, .. }
        | ForgeConnectorFrame::InvocationHandoff { request_id, .. } => request_id.clone(),
    }
}
