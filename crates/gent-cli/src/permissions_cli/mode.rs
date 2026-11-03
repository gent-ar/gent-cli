use std::path::PathBuf;

use gent_protocol::PermissionPolicyFrame;
use gent_types::{PermissionMode, PolicyRecord};

use super::{current_for, exchange, policy};

pub(super) fn require_provider_binding(
    request: &PermissionPolicyFrame,
    capabilities: &gent_types::CapabilitySet,
) -> Result<(), Box<dyn std::error::Error>> {
    if matches!(request, PermissionPolicyFrame::Save { policy, .. } if policy.mode.is_unattended())
        && !capabilities.0.iter().any(|capability| {
            capability == gent_protocol::PERMISSION_POLICY_PROVIDER_BINDING_CAPABILITY
        })
    {
        return Err(
            "gentd does not support provider-bound broad permission policies; upgrade gentd".into(),
        );
    }
    Ok(())
}

pub(crate) async fn set_mode(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    workspace_id: String,
    mode: PermissionMode,
    bypass_consent: bool,
    expected_provider: Option<gent_types::AgentChatProvider>,
) -> Result<PolicyRecord, Box<dyn std::error::Error>> {
    if mode == PermissionMode::Bypass && !bypass_consent {
        return Err("changing to bypass mode requires explicit confirmation".into());
    }
    let current = current_for(data_dir.clone(), no_autostart, workspace_id.clone()).await?;
    let revision = current.as_ref().map_or(1, |value| value.revision + 1);
    let policy = policy(
        &workspace_id,
        revision,
        mode,
        current
            .as_ref()
            .map_or_else(Vec::new, |value| value.allowed_tools.clone()),
        current
            .as_ref()
            .map_or_else(Vec::new, |value| value.allowed_categories.clone()),
    );
    exchange(
        data_dir,
        no_autostart,
        PermissionPolicyFrame::Save {
            request_id: uuid::Uuid::new_v4().to_string(),
            policy: policy.clone(),
            bypass_consent,
            expected_provider: mode.is_unattended().then_some(expected_provider).flatten(),
        },
    )
    .await
    .and_then(|frame| match frame {
        PermissionPolicyFrame::Saved { policy, .. } => Ok(policy),
        _ => Err("daemon did not save a permission policy".into()),
    })
}
