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
        workspace_id: "workspace-1".into(),
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
fn current_policy_reply_stays_bound_to_the_requested_workspace() {
    let request = PermissionPolicyFrame::Current {
        request_id: "request-1".into(),
        workspace_id: "workspace-1".into(),
    };
    let response = PermissionPolicyFrame::CurrentPolicy {
        request_id: "request-1".into(),
        policy: Some(PolicyRecord {
            policy_id: "policy-1".into(),
            workspace_id: "another-workspace".into(),
            scope: PolicyScope::ProviderPermissions,
            revision: 1,
            mode: PermissionMode::AskEveryTime,
            allowed_tools: vec![],
            allowed_categories: vec![],
        }),
    };
    assert!(!valid_reply(&request, &response));
}

#[test]
fn respond_answers_one_exact_pending_decision_with_a_typed_choice() {
    let args = Args::try_parse_from([
        "gent",
        "permissions",
        "respond",
        "--conversation-id",
        "conversation-1",
        "--run-id",
        "run-1",
        "--decision-id",
        "decision-1",
        "--decision",
        "approve-once",
    ])
    .unwrap();
    assert!(matches!(
        args.command,
        Some(CommandLine::Permissions {
            action: PermissionCommand::Respond(super::PermissionRespondArgs {
                decision: super::PermissionDecisionArgument::ApproveOnce,
                ..
            })
        })
    ));
}

#[test]
fn pending_command_is_scoped_to_one_conversation_run() {
    let args = Args::try_parse_from([
        "gent",
        "permissions",
        "pending",
        "--conversation-id",
        "conversation-1",
        "--run-id",
        "run-1",
    ])
    .unwrap();
    assert!(matches!(
        args.command,
        Some(CommandLine::Permissions {
            action: PermissionCommand::Pending(_)
        })
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn set_saves_the_next_revision_for_the_resolved_workspace() {
    use gent_protocol::{
        Hello, Negotiated, PERMISSION_POLICY_CAPABILITY, WORKSPACE_GIT_CAPABILITY, WireFrame,
        WorkspaceGitFrame, read_frame, read_json_frame, write_frame, write_json_frame,
    };
    use gent_types::{CapabilitySet, PROTOCOL_MAX};

    let directory = tempfile::tempdir().unwrap();
    let listener = tokio::net::UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    let saved = std::sync::Arc::new(std::sync::Mutex::new(None));
    let recorded = std::sync::Arc::clone(&saved);
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert!(matches!(
                read_frame(&mut stream).await.unwrap(),
                WireFrame::Hello(Hello { .. })
            ));
            write_frame(
                &mut stream,
                &WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet(vec![
                        PERMISSION_POLICY_CAPABILITY.into(),
                        WORKSPACE_GIT_CAPABILITY.into(),
                    ]),
                }),
            )
            .await
            .unwrap();
            let raw: serde_json::Value = read_json_frame(&mut stream).await.unwrap();
            let reply = if let Ok(WorkspaceGitFrame::ResolveRequest { request_id, .. }) =
                serde_json::from_value(raw.clone())
            {
                serde_json::to_value(WorkspaceGitFrame::Resolved {
                    request_id,
                    workspace_id: "workspace-ws2".into(),
                    canonical_path: "/tmp/ws2".into(),
                })
                .unwrap()
            } else {
                match serde_json::from_value(raw).unwrap() {
                    PermissionPolicyFrame::Current {
                        request_id,
                        workspace_id,
                    } => serde_json::to_value(PermissionPolicyFrame::CurrentPolicy {
                        request_id,
                        policy: Some(PolicyRecord {
                            policy_id: "default".into(),
                            workspace_id,
                            scope: PolicyScope::ProviderPermissions,
                            revision: 1,
                            mode: PermissionMode::AskEveryTime,
                            allowed_tools: vec![],
                            allowed_categories: vec![],
                        }),
                    })
                    .unwrap(),
                    PermissionPolicyFrame::Save {
                        request_id, policy, ..
                    } => {
                        *recorded.lock().unwrap() = Some(policy.clone());
                        serde_json::to_value(PermissionPolicyFrame::Saved { request_id, policy })
                            .unwrap()
                    }
                    _ => unreachable!(),
                }
            };
            write_json_frame(&mut stream, &reply).await.unwrap();
        }
    });
    let args = Args::try_parse_from([
        "gent",
        "permissions",
        "set",
        "--workspace",
        "/tmp/ws2",
        "--mode",
        "autonomous",
    ])
    .unwrap();
    let Some(CommandLine::Permissions { action }) = args.command else {
        panic!("permissions command");
    };
    super::execute(Some(directory.path().into()), true, action)
        .await
        .unwrap();
    let saved = saved.lock().unwrap().clone().unwrap();
    assert_eq!(
        (saved.workspace_id.as_str(), saved.revision, saved.mode),
        ("workspace-ws2", 2, PermissionMode::Autonomous)
    );
}
