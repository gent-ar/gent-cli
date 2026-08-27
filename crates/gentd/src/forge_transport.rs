use gent_protocol::{FORGE_CONNECTORS_CAPABILITY, ForgeConnectorFrame, write_json_frame};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::io::AsyncWrite;

use crate::{api::RuntimeApi, transport::write_error};

pub(crate) async fn dispatch<S, R>(
    stream: &mut S,
    runtime: &R,
    capabilities: &CapabilitySet,
    raw: &Value,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    R: RuntimeApi,
{
    if !capabilities
        .0
        .iter()
        .any(|item| item == FORGE_CONNECTORS_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<ForgeConnectorFrame>(raw.clone()) else {
        return Ok(false);
    };
    if frame.validate().is_err() || !is_request(&frame) {
        write_error(stream, "invalidForge", "Forge request is invalid").await?;
        return Ok(true);
    }
    match runtime.forge_connectors(frame) {
        Ok(reply) if reply.validate().is_ok() && is_response(&reply) => {
            write_json_frame(stream, &reply).await?;
        }
        Ok(_) => {
            write_error(
                stream,
                "invalidForge",
                "Forge runtime returned an invalid response",
            )
            .await?;
        }
        Err(message) => write_error(stream, "forgeRejected", &message).await?,
    }
    Ok(true)
}

fn is_request(frame: &ForgeConnectorFrame) -> bool {
    matches!(
        frame,
        ForgeConnectorFrame::ListRequest { .. }
            | ForgeConnectorFrame::GetRequest { .. }
            | ForgeConnectorFrame::CreateRequest { .. }
            | ForgeConnectorFrame::SetEnabledRequest { .. }
            | ForgeConnectorFrame::InvokeRequest { .. }
    )
}

fn is_response(frame: &ForgeConnectorFrame) -> bool {
    matches!(
        frame,
        ForgeConnectorFrame::List { .. }
            | ForgeConnectorFrame::Get { .. }
            | ForgeConnectorFrame::Created { .. }
            | ForgeConnectorFrame::SetEnabled { .. }
            | ForgeConnectorFrame::InvocationHandoff { .. }
    )
}
