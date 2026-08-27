use gent_protocol::{AUTOMATIONS_CAPABILITY, AutomationFrame, write_json_frame};
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
        .any(|item| item == AUTOMATIONS_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<AutomationFrame>(raw.clone()) else {
        return Ok(false);
    };
    if !matches!(
        frame,
        AutomationFrame::ListRequest { .. }
            | AutomationFrame::CreateRequest { .. }
            | AutomationFrame::RunRequest { .. }
            | AutomationFrame::RunsRequest { .. }
    ) || frame.validate().is_err()
    {
        write_error(stream, "invalidAutomation", "automation request is invalid").await?;
        return Ok(true);
    }
    match runtime.automations(frame) {
        Ok(reply)
            if matches!(
                reply,
                AutomationFrame::List { .. }
                    | AutomationFrame::Created { .. }
                    | AutomationFrame::RunAccepted { .. }
                    | AutomationFrame::Runs { .. }
            ) && reply.validate().is_ok() =>
        {
            write_json_frame(stream, &reply).await?;
        }
        Ok(_) => {
            write_error(
                stream,
                "invalidAutomation",
                "automation runtime returned an invalid response",
            )
            .await?;
        }
        Err(message) => write_error(stream, "automationRejected", &message).await?,
    }
    Ok(true)
}
