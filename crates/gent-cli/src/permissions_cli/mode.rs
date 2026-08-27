use std::path::PathBuf;

use gent_protocol::PermissionPolicyFrame;
use gent_types::{PermissionMode, PolicyRecord};

use super::{current_for, exchange, policy};

pub(crate) async fn set_mode(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    workspace_id: String,
    mode: PermissionMode,
    bypass_consent: bool,
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
        },
    )
    .await
    .and_then(|frame| match frame {
        PermissionPolicyFrame::Saved { policy, .. } => Ok(policy),
        _ => Err("daemon did not save a permission policy".into()),
    })
}
