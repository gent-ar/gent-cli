//! Capability-gated transport for one read-only durable update maintenance record.

use gent_protocol::{RUNTIME_MAINTENANCE_CAPABILITY, RuntimeMaintenanceFrame, write_json_frame};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::io::AsyncWrite;

use crate::{api::RuntimeApi, transport::write_error};

/// Dispatches only a negotiated read-only maintenance request.
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
        .any(|capability| capability == RUNTIME_MAINTENANCE_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<RuntimeMaintenanceFrame>(raw.clone()) else {
        return Ok(false);
    };
    let RuntimeMaintenanceFrame::Request(request) = frame else {
        write_error(
            stream,
            "invalidRuntimeMaintenance",
            "maintenance reports are server-only",
        )
        .await?;
        return Ok(true);
    };
    match runtime.runtime_maintenance(request) {
        Ok(report) => {
            write_json_frame(stream, &RuntimeMaintenanceFrame::Report(Box::new(report))).await?;
        }
        Err(message) => write_error(stream, "runtimeMaintenanceUnavailable", &message).await?,
    }
    Ok(true)
}
