use std::path::PathBuf;

use gent_protocol::{
    AUTOMATIONS_CAPABILITY, AutomationFrame, WireFrame, read_json_frame, write_json_frame,
};
use gent_types::{AutomationDefinition, AutomationId, AutomationRun};
use serde_json::Value;

use crate::local_ipc::connect_and_negotiate;

pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    action: crate::AutomationCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    let value = match action {
        crate::AutomationCommand::List { workspace_id } => {
            serde_json::to_value(list(data_dir, no_autostart, workspace_id).await?)?
        }
        crate::AutomationCommand::Create { definition } => {
            let definition = serde_json::from_str(&definition)?;
            serde_json::to_value(create(data_dir, no_autostart, definition).await?)?
        }
        crate::AutomationCommand::Run { automation_id } => {
            serde_json::to_value(run(data_dir, no_autostart, AutomationId(automation_id)).await?)?
        }
        crate::AutomationCommand::Runs {
            automation_id,
            limit,
        } => serde_json::to_value(
            runs(data_dir, no_autostart, AutomationId(automation_id), limit).await?,
        )?,
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

pub(crate) async fn list(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    workspace_id: String,
) -> Result<Vec<AutomationDefinition>, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|capability| capability == AUTOMATIONS_CAPABILITY)
    {
        return Err("daemon does not support Gent automation catalogs; upgrade gentd".into());
    }
    let request_id = uuid::Uuid::new_v4().to_string();
    write_json_frame(
        &mut stream,
        &AutomationFrame::ListRequest {
            request_id: request_id.clone(),
            workspace_id: workspace_id.clone(),
        },
    )
    .await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw.clone()) {
        return Err(message.into());
    }
    match serde_json::from_value(raw)? {
        AutomationFrame::List {
            request_id: reply_id,
            workspace_id: reply_workspace,
            automations,
        } if reply_id == request_id
            && reply_workspace == workspace_id
            && automations
                .iter()
                .all(|item| item.workspace_id == workspace_id) =>
        {
            Ok(automations)
        }
        _ => Err("daemon returned an invalid automation catalog".into()),
    }
}

pub(crate) async fn create(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    definition: AutomationDefinition,
) -> Result<AutomationDefinition, Box<dyn std::error::Error>> {
    let request_id = uuid::Uuid::new_v4().to_string();
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    require_capability(&capabilities)?;
    write_json_frame(
        &mut stream,
        &AutomationFrame::CreateRequest {
            request_id: request_id.clone(),
            definition,
        },
    )
    .await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw.clone()) {
        return Err(message.into());
    }
    match serde_json::from_value(raw)? {
        AutomationFrame::Created {
            request_id: reply_id,
            definition,
        } if reply_id == request_id => Ok(definition),
        _ => Err("daemon returned an invalid automation create response".into()),
    }
}

pub(crate) async fn run(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    automation_id: AutomationId,
) -> Result<AutomationFrame, Box<dyn std::error::Error>> {
    exchange(
        data_dir,
        no_autostart,
        AutomationFrame::RunRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            automation_id,
        },
    )
    .await
}

pub(crate) async fn runs(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    automation_id: AutomationId,
    limit: u16,
) -> Result<Vec<AutomationRun>, Box<dyn std::error::Error>> {
    let request_id = uuid::Uuid::new_v4().to_string();
    match exchange(
        data_dir,
        no_autostart,
        AutomationFrame::RunsRequest {
            request_id: request_id.clone(),
            automation_id,
            limit,
        },
    )
    .await?
    {
        AutomationFrame::Runs {
            request_id: reply_id,
            runs,
            ..
        } if reply_id == request_id => Ok(runs),
        _ => Err("daemon returned an invalid automation run response".into()),
    }
}

async fn exchange(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    frame: AutomationFrame,
) -> Result<AutomationFrame, Box<dyn std::error::Error>> {
    let request_id = match &frame {
        AutomationFrame::RunRequest { request_id, .. }
        | AutomationFrame::RunsRequest { request_id, .. } => request_id.clone(),
        _ => return Err("unsupported automation exchange".into()),
    };
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    require_capability(&capabilities)?;
    write_json_frame(&mut stream, &frame).await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw.clone()) {
        return Err(message.into());
    }
    let response: AutomationFrame = serde_json::from_value(raw)?;
    let response_id = match &response {
        AutomationFrame::RunAccepted { request_id, .. }
        | AutomationFrame::Runs { request_id, .. } => request_id,
        _ => return Err("daemon returned an invalid automation response".into()),
    };
    if *response_id != request_id {
        return Err("daemon returned a mismatched automation response".into());
    }
    Ok(response)
}

fn require_capability(
    capabilities: &gent_types::CapabilitySet,
) -> Result<(), Box<dyn std::error::Error>> {
    if capabilities
        .0
        .iter()
        .any(|value| value == AUTOMATIONS_CAPABILITY)
    {
        Ok(())
    } else {
        Err("daemon does not support Gent automations; upgrade gentd".into())
    }
}
