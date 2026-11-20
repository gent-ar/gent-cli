use std::path::PathBuf;

use gent_protocol::{PERMISSION_POLICY_CAPABILITY, PermissionPolicyFrame, write_json_frame};
use gent_types::{PermissionCategory, PermissionMode, PolicyRecord, PolicyScope};
use serde_json::Value;

use super::conversions::valid_reply;
use crate::local_ipc::connect_and_negotiate;

pub(in crate::permissions_cli) fn policy(
    workspace_id: &str,
    revision: u64,
    mode: PermissionMode,
    allowed_tools: Vec<String>,
    allowed_categories: Vec<PermissionCategory>,
    allowed_commands: Vec<gent_types::CommandGrant>,
) -> PolicyRecord {
    PolicyRecord {
        policy_id: format!("permission-policy-{}", uuid::Uuid::new_v4()),
        workspace_id: workspace_id.into(),
        scope: PolicyScope::ProviderPermissions,
        revision,
        mode,
        containment: gent_types::Containment::Sandboxed,
        allowed_tools,
        allowed_categories,
        allowed_commands,
    }
}

pub(in crate::permissions_cli) async fn exchange(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    request: PermissionPolicyFrame,
) -> Result<PermissionPolicyFrame, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|capability| capability == PERMISSION_POLICY_CAPABILITY)
    {
        return Err("gentd does not support permission policy; upgrade gentd".into());
    }
    write_json_frame(&mut stream, &request).await?;
    let raw: Value = crate::local_ipc::read_reply(&mut stream).await?;
    let response = serde_json::from_value(raw)
        .map_err(|_| "daemon did not return a permission policy frame")?;
    valid_reply(&request, &response)
        .then_some(response)
        .ok_or_else(|| {
            "daemon returned a permission policy response with a different request".into()
        })
}
