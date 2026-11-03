use gent_types::{
    PermissionDecisionRequest, PermissionDecisionResponse, PermissionDecisionResponseKind,
    PolicyRecord, PolicyScope,
};
use sha2::{Digest, Sha256};

pub(crate) fn transition(
    policy: &PolicyRecord,
    pending: &PermissionDecisionRequest,
    response: &PermissionDecisionResponse,
) -> Result<Option<PolicyRecord>, &'static str> {
    if pending.binding != response.binding {
        return Err("permission response binding is stale");
    }
    if policy.scope != PolicyScope::ProviderPermissions
        || policy.policy_id != pending.binding.policy_id
        || policy.revision != pending.binding.policy_revision
    {
        return Err("permission policy is stale");
    }
    let mut revised = policy.clone();
    match response.response {
        PermissionDecisionResponseKind::Deny | PermissionDecisionResponseKind::ApproveOnce => {
            return Ok(None);
        }
        PermissionDecisionResponseKind::ApproveExactTool => {
            revised
                .allowed_tools
                .push(pending.request.tool_name.clone());
            revised.allowed_tools.sort();
            revised.allowed_tools.dedup();
        }
        PermissionDecisionResponseKind::ApproveCategory => {
            revised.allowed_categories.push(pending.request.category);
            revised.allowed_categories.sort();
            revised.allowed_categories.dedup();
        }
    }
    if revised == *policy {
        return Ok(None);
    }
    revised.revision = policy
        .revision
        .checked_add(1)
        .ok_or("permission policy revision overflow")?;
    revised.policy_id = format!(
        "provider-permissions-v{}-{:x}",
        revised.revision,
        Sha256::digest(
            format!(
                "{}\0{}\0{}",
                policy.workspace_id, policy.policy_id, revised.revision
            )
            .as_bytes()
        )
    );
    Ok(Some(revised))
}

#[cfg(test)]
#[path = "permission_policy_grant_tests.rs"]
mod tests;
