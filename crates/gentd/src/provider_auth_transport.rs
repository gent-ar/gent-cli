//! Capability-gated local transport for secret-free provider authentication.
//!
//! The shipped observer never advertises this capability. This boundary is deliberately inert
//! until a future authority composition supplies a reviewed provider-auth runtime port.

use gent_protocol::{PROVIDER_AUTH_CAPABILITY, ProviderAuthFrame, write_json_frame};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::io::AsyncWrite;

use crate::{api::RuntimeApi, transport::write_error};

/// Decodes and responds to one provider-auth request after capability negotiation.
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
        .any(|capability| capability == PROVIDER_AUTH_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<ProviderAuthFrame>(raw.clone()) else {
        return Ok(false);
    };
    if frame.validate().is_err() {
        write_error(
            stream,
            "invalidProviderAuth",
            "provider authentication request is invalid",
        )
        .await?;
        return Ok(true);
    }
    if !is_client_request(&frame) {
        write_error(
            stream,
            "invalidProviderAuth",
            "provider authentication response frames are server-only",
        )
        .await?;
        return Ok(true);
    }
    match runtime.provider_auth(frame) {
        Ok(response) => write_json_frame(stream, &response).await?,
        Err(message) => write_error(stream, "providerAuthRejected", &message).await?,
    }
    Ok(true)
}

fn is_client_request(frame: &ProviderAuthFrame) -> bool {
    matches!(
        frame,
        ProviderAuthFrame::StatusRequest { .. }
            | ProviderAuthFrame::LoginRequest { .. }
            | ProviderAuthFrame::SelectMethod { .. }
            | ProviderAuthFrame::Cancel { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::is_client_request;
    use gent_protocol::ProviderAuthFrame;
    use gent_types::ProviderAuthProvider;

    fn request() -> ProviderAuthFrame {
        ProviderAuthFrame::StatusRequest {
            request_id: "request-1".into(),
            provider: ProviderAuthProvider::Claude,
        }
    }

    #[test]
    fn only_client_request_variants_reach_the_runtime_port() {
        assert!(is_client_request(&request()));
        assert!(is_client_request(&ProviderAuthFrame::LoginRequest {
            request_id: "request-1".into(),
            provider: ProviderAuthProvider::Claude,
        }));
    }
}
