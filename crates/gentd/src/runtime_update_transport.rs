//! Capability-gated transport for the daemon's read-only cached update report.

use gent_protocol::{RUNTIME_UPDATE_CHECK_CAPABILITY, RuntimeUpdateCheckFrame, write_json_frame};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::io::AsyncWrite;

use crate::{api::RuntimeApi, transport::write_error};

/// Dispatches only a client update-check request after capability negotiation.
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
        .any(|capability| capability == RUNTIME_UPDATE_CHECK_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<RuntimeUpdateCheckFrame>(raw.clone()) else {
        return Ok(false);
    };
    let RuntimeUpdateCheckFrame::Request(request) = frame else {
        write_error(
            stream,
            "invalidRuntimeUpdateCheck",
            "runtime update reports are server-only",
        )
        .await?;
        return Ok(true);
    };
    match runtime.runtime_update_check(request) {
        Ok(report) => write_json_frame(stream, &RuntimeUpdateCheckFrame::Report(report)).await?,
        Err(message) => write_error(stream, "runtimeUpdateUnavailable", &message).await?,
    }
    Ok(true)
}
