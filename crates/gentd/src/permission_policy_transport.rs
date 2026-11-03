//! Capability-gated local transport for permission-policy settings.

use gent_protocol::{PERMISSION_POLICY_CAPABILITY, PermissionPolicyFrame, write_json_frame};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::io::AsyncWrite;

use crate::{api::RuntimeApi, transport::write_error};

/// Decodes and responds to one policy frame only after successful negotiation.
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
        .any(|capability| capability == PERMISSION_POLICY_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<PermissionPolicyFrame>(raw.clone()) else {
        return Ok(false);
    };
    if matches!(
        &frame,
        PermissionPolicyFrame::Save {
            expected_provider: Some(_),
            ..
        }
    ) && !capabilities.0.iter().any(|capability| {
        capability == gent_protocol::PERMISSION_POLICY_PROVIDER_BINDING_CAPABILITY
    }) {
        write_error(stream, "unsupportedCapability", "provider-bound permission saves require permission-policy-provider-binding-v1 negotiation").await?;
        return Ok(true);
    }
    if !matches!(
        frame,
        PermissionPolicyFrame::Current { .. } | PermissionPolicyFrame::Save { .. }
    ) {
        write_error(
            stream,
            "invalidPermissionPolicy",
            "policy response frames are server-only",
        )
        .await?;
        return Ok(true);
    }
    // Reviewed resource preparation can hash files and verify OS signatures.
    // Retain admission inside the blocking worker even if this viewer disconnects.
    let permit = match runtime
        .runtime_owner_control_port()
        .map(|owner| owner.acquire_operation())
        .transpose()
    {
        Ok(permit) => permit,
        Err(message) => {
            write_error(stream, "runtimeOwnerNotReady", &message).await?;
            return Ok(true);
        }
    };
    let authority = runtime.clone();
    let response = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        authority.permission_policy(frame)
    })
    .await
    .unwrap_or_else(|_| Err("permission policy preparation worker stopped unexpectedly".into()));
    match response {
        Ok(response) => write_json_frame(stream, &response).await?,
        Err(message) => write_error(stream, "permissionPolicyRejected", &message).await?,
    }
    Ok(true)
}
