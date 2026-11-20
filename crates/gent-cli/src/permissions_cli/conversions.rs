use gent_protocol::PermissionPolicyFrame;
use gent_types::{PermissionCategory, PermissionMode, PolicyScope};

use super::{PermissionCategoryArgument, PermissionDecisionArgument, PermissionModeArgument};

pub(super) fn command_grants(
    values: &[String],
) -> Result<Vec<gent_types::CommandGrant>, crate::cli_error::CliError> {
    let mut grants = values
        .iter()
        .map(|value| {
            let (tool, pattern) = value.split_once('=').unwrap_or(("", value));
            gent_types::parse_command_pattern(tool, pattern).map_err(|error| {
                crate::cli_error::CliError::new(
                    crate::cli_error::Failure::Rejected,
                    format!("--allow-command {value}: {error} ({})", error.code()),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    grants.sort();
    grants.dedup();
    Ok(grants)
}

pub(super) fn valid_reply(
    request: &PermissionPolicyFrame,
    response: &PermissionPolicyFrame,
) -> bool {
    match (request, response) {
        (
            PermissionPolicyFrame::Current {
                request_id,
                workspace_id,
            },
            PermissionPolicyFrame::CurrentPolicy {
                request_id: reply,
                policy,
            },
        ) => {
            reply == request_id
                && policy.as_ref().is_none_or(|policy| {
                    policy.workspace_id == *workspace_id
                        && policy.scope == PolicyScope::ProviderPermissions
                })
        }
        (
            PermissionPolicyFrame::Save {
                request_id, policy, ..
            },
            PermissionPolicyFrame::Saved {
                request_id: reply,
                policy: saved,
            },
        ) => reply == request_id && saved == policy,
        _ => false,
    }
}

impl From<PermissionModeArgument> for PermissionMode {
    fn from(value: PermissionModeArgument) -> Self {
        match value {
            PermissionModeArgument::AskEveryTime => Self::AskEveryTime,
            PermissionModeArgument::AutoAcceptEdits => Self::AutoAcceptEdits,
            PermissionModeArgument::Autonomous => Self::Autonomous,
            PermissionModeArgument::Bypass => Self::Bypass,
        }
    }
}

impl From<PermissionDecisionArgument> for gent_types::PermissionDecisionResponseKind {
    fn from(value: PermissionDecisionArgument) -> Self {
        match value {
            PermissionDecisionArgument::Deny => Self::Deny,
            PermissionDecisionArgument::ApproveOnce => Self::ApproveOnce,
            PermissionDecisionArgument::ApproveExactTool => Self::ApproveExactTool,
            PermissionDecisionArgument::ApproveCategory => Self::ApproveCategory,
            PermissionDecisionArgument::ApproveCommandPattern => Self::ApproveCommandPattern,
        }
    }
}

impl From<PermissionCategoryArgument> for PermissionCategory {
    fn from(value: PermissionCategoryArgument) -> Self {
        match value {
            PermissionCategoryArgument::Read => Self::Read,
            PermissionCategoryArgument::Edit => Self::Edit,
            PermissionCategoryArgument::Command => Self::Command,
            PermissionCategoryArgument::Network => Self::Network,
            PermissionCategoryArgument::Provider => Self::Provider,
        }
    }
}
