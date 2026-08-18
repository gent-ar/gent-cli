use clap::Parser;
use gent_protocol::PermissionPolicyFrame;
use gent_types::{PermissionMode, PolicyRecord, PolicyScope};

use super::{PermissionCommand, PermissionModeArgument, valid_reply};
use crate::{Args, CommandLine};

#[test]
fn bypass_is_a_mode_with_a_one_time_configuration_confirmation() {
    let args = Args::try_parse_from(["gent", "permissions", "set", "--mode", "bypass"]).unwrap();
    assert!(matches!(
        args.command,
        Some(CommandLine::Permissions { .. })
    ));
    let args = Args::try_parse_from([
        "gent",
        "permissions",
        "set",
        "--mode",
        "bypass",
        "--consent-bypass",
    ])
    .unwrap();
    assert!(matches!(
        args.command,
        Some(CommandLine::Permissions { .. })
    ));
}

#[test]
fn autonomous_is_a_durable_permission_command_not_a_chat_mode_alias() {
    let args =
        Args::try_parse_from(["gent", "permissions", "set", "--mode", "autonomous"]).unwrap();
    assert!(matches!(
        args.command,
        Some(CommandLine::Permissions {
            action: PermissionCommand::Set(_)
        })
    ));
    assert_eq!(
        PermissionMode::from(PermissionModeArgument::Autonomous),
        PermissionMode::Autonomous
    );
}

#[test]
fn autonomous_save_requires_the_exact_revisioned_policy_reply() {
    let policy = PolicyRecord {
        policy_id: "policy-1".into(),
        workspace_id: super::SETTINGS_WORKSPACE_ID.into(),
        scope: PolicyScope::ProviderPermissions,
        revision: 2,
        mode: PermissionMode::Autonomous,
        allowed_tools: vec!["workspace.read".into()],
        allowed_categories: vec![],
    };
    let request = PermissionPolicyFrame::Save {
        request_id: "request-1".into(),
        policy: policy.clone(),
        bypass_consent: false,
    };
    let matching = PermissionPolicyFrame::Saved {
        request_id: "request-1".into(),
        policy,
    };
    let wrong_request = PermissionPolicyFrame::Saved {
        request_id: "request-2".into(),
        policy: match &matching {
            PermissionPolicyFrame::Saved { policy, .. } => policy.clone(),
            _ => unreachable!(),
        },
    };
    assert!(valid_reply(&request, &matching));
    assert!(!valid_reply(&request, &wrong_request));
}

#[test]
fn current_policy_reply_stays_bound_to_the_settings_workspace() {
    let request = PermissionPolicyFrame::Current {
        request_id: "request-1".into(),
        workspace_id: super::SETTINGS_WORKSPACE_ID.into(),
    };
    let response = PermissionPolicyFrame::CurrentPolicy {
        request_id: "request-1".into(),
        policy: Some(PolicyRecord {
            policy_id: "policy-1".into(),
            workspace_id: "another-workspace".into(),
            scope: PolicyScope::ProviderPermissions,
            revision: 1,
            mode: PermissionMode::Plan,
            allowed_tools: vec![],
            allowed_categories: vec![],
        }),
    };
    assert!(!valid_reply(&request, &response));
}
