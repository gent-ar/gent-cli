use gent_ports::{
    AgentChatPromptLedger, ClaurstDrainBatch, ClaurstPermissionReply, ClaurstPermissionRequest,
    PendingPermissionLedger, PolicyLedger, TranscriptLedger,
};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_testkit::FakePrivateClaurstBridge;
use gent_types::{
    AgentChatConversationId, AgentChatMode, AgentChatPromptCreate, AgentChatPromptDisposition,
    AgentChatRequestId, AgentChatRunId, CapabilitySet, HostEpoch, NormalizedTranscriptKind,
    PermissionCategory, PermissionDenialReason, PermissionMode, PolicyRecord, PolicyScope,
    ReceiptId,
};

use crate::private_claurst_ingress::PrivateClaurstIngress;
use crate::private_claurst_ingress_tests::{
    binding, checkpoint, prepared_ledger_in_mode, start_request,
};

fn permission(
    id: &str,
    tool: &str,
    category: PermissionCategory,
    input: serde_json::Value,
) -> ClaurstPermissionRequest {
    ClaurstPermissionRequest {
        request_id: id.into(),
        tool_use_id: format!("tool-{id}"),
        tool_name: tool.into(),
        category,
        input: Some(input),
    }
}

async fn prepare_permissions(
    mode: AgentChatMode,
    policy: PermissionMode,
    permissions: Vec<ClaurstPermissionRequest>,
) -> (
    SqliteLedger,
    FakePrivateClaurstBridge,
    PrivateClaurstIngress<SqliteLedger, FakePrivateClaurstBridge>,
) {
    let ledger = prepared_ledger_in_mode(mode);
    let launched = PolicyRecord {
        policy_id: "policy-a".into(),
        workspace_id: "workspace-a".into(),
        scope: PolicyScope::ProviderPermissions,
        revision: 1,
        mode: policy,
        containment: gent_types::Containment::Sandboxed,
        allowed_tools: vec![],
        allowed_categories: vec![],
    };
    ledger.save_policy(&launched).unwrap();
    gent_ports::AgentChatConversationConfigLedger::save_conversation_config(
        &ledger,
        &gent_types::AgentChatConversationConfigRecord {
            conversation_id: AgentChatConversationId("conversation-a".into()),
            revision: 1,
            system_prompt: None,
            append_system_prompt: false,
            max_turns: None,
            thinking_enabled: None,
            disallowed_tools: vec![],
            containment: gent_types::Containment::Sandboxed,
        },
    )
    .unwrap();
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("request-permission".into()),
            receipt_id: ReceiptId("receipt-permission".into()),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            disposition: AgentChatPromptDisposition::Send,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: "change something".into(),
        })
        .unwrap();
    let bridge = FakePrivateClaurstBridge::default();
    bridge.bind_turn_policy(launched);
    let binding = binding();
    bridge.push_start_binding(binding.clone());
    for permission in permissions {
        bridge.push_batch(ClaurstDrainBatch {
            facts: vec![],
            permissions: vec![permission],
            checkpoint: Some(checkpoint(0)),
            session_binding: Some(binding.clone()),
            terminal: None,
        });
    }
    let mut ingress = PrivateClaurstIngress::new(
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger.clone(),
        bridge.clone(),
        "daemon-a".into(),
    );
    let mut request = start_request();
    request.turn_id = saved.message.turn_id;
    ingress.start(request, HostEpoch(1)).await.unwrap();
    while bridge.has_pending_batches() {
        ingress
            .drain(&binding.source_id, HostEpoch(1))
            .await
            .unwrap();
    }
    (ledger, bridge, ingress)
}

async fn drain_permissions(
    mode: AgentChatMode,
    policy: PermissionMode,
    permissions: Vec<ClaurstPermissionRequest>,
) -> (SqliteLedger, FakePrivateClaurstBridge) {
    let (ledger, bridge, _) = prepare_permissions(mode, policy, permissions).await;
    (ledger, bridge)
}

fn pending(ledger: &SqliteLedger) -> bool {
    ledger
        .pending_permission(
            &AgentChatConversationId("conversation-a".into()),
            &AgentChatRunId("run-a".into()),
        )
        .unwrap()
        .is_some()
}

#[tokio::test]
async fn an_ask_mode_change_is_denied_by_gent_with_a_typed_notice() {
    let (ledger, bridge) = drain_permissions(
        AgentChatMode::Ask,
        PermissionMode::Bypass,
        vec![permission(
            "edit-1",
            "Edit",
            PermissionCategory::Edit,
            serde_json::json!({"file_path": "/workspace-a/src/app.py"}),
        )],
    )
    .await;
    assert_eq!(
        bridge.permission_replies(),
        [("edit-1".to_owned(), ClaurstPermissionReply::Deny)]
    );
    assert!(!pending(&ledger));
    let notices = ledger
        .normalized_transcript_page(&AgentChatConversationId("conversation-a".into()), 0, 8)
        .unwrap()
        .events
        .into_iter()
        .filter(|event| event.kind == NormalizedTranscriptKind::Notice)
        .map(|event| event.text)
        .collect::<Vec<_>>();
    assert_eq!(notices, [PermissionDenialReason::ReadOnlyMode.notice()]);
}

#[tokio::test]
async fn agent_mode_workspace_reads_and_edits_do_not_wait_for_the_user() {
    let (ledger, bridge) = drain_permissions(
        AgentChatMode::Agent,
        PermissionMode::AutoAcceptEdits,
        vec![
            permission(
                "glob-1",
                "Glob",
                PermissionCategory::Read,
                serde_json::json!({"pattern": "**/*"}),
            ),
            permission(
                "edit-1",
                "Edit",
                PermissionCategory::Edit,
                serde_json::json!({"file_path": "/workspace-a/src/app.py"}),
            ),
        ],
    )
    .await;
    assert_eq!(
        bridge.permission_replies(),
        [
            ("glob-1".to_owned(), ClaurstPermissionReply::AllowOnce),
            ("edit-1".to_owned(), ClaurstPermissionReply::AllowOnce),
        ]
    );
    assert!(!pending(&ledger));
}

#[tokio::test]
async fn a_sandboxed_autonomous_command_waits_for_the_user_without_an_os_sandbox() {
    let (ledger, bridge) = drain_permissions(
        AgentChatMode::Agent,
        PermissionMode::Autonomous,
        vec![permission(
            "bash-1",
            "Bash",
            PermissionCategory::Command,
            serde_json::json!({"command": "python3 -m src.cli show"}),
        )],
    )
    .await;
    assert!(bridge.permission_replies().is_empty());
    assert!(pending(&ledger));
}

#[tokio::test]
async fn a_sandboxed_bypass_workspace_edit_waits_without_live_containment() {
    let (ledger, bridge) = drain_permissions(
        AgentChatMode::Agent,
        PermissionMode::Bypass,
        vec![permission(
            "edit-1",
            "Edit",
            PermissionCategory::Edit,
            serde_json::json!({"file_path": "/workspace-a/src/app.py"}),
        )],
    )
    .await;
    assert!(bridge.permission_replies().is_empty());
    assert!(pending(&ledger));
}

#[tokio::test]
async fn claurst_commits_canonical_grant_and_answers_after_a_newer_revision() {
    use gent_types::{PermissionDecisionResponse, PermissionDecisionResponseKind};
    for stale in [false, true] {
        let (ledger, bridge, ingress) = prepare_permissions(
            AgentChatMode::Agent,
            PermissionMode::AskEveryTime,
            vec![permission(
                "command",
                "Bash",
                PermissionCategory::Command,
                serde_json::json!({"command":"fixture"}),
            )],
        )
        .await;
        let request = ledger
            .pending_permission(
                &AgentChatConversationId("conversation-a".into()),
                &AgentChatRunId("run-a".into()),
            )
            .unwrap()
            .unwrap();
        if stale {
            let mut policy = ledger
                .current_policy("workspace-a", PolicyScope::ProviderPermissions)
                .unwrap()
                .unwrap();
            policy.policy_id = "newer-policy".into();
            policy.revision = 2;
            ledger.save_policy(&policy).unwrap();
        }
        let response = PermissionDecisionResponse {
            binding: request.binding.clone(),
            response: if stale {
                PermissionDecisionResponseKind::ApproveOnce
            } else {
                PermissionDecisionResponseKind::ApproveExactTool
            },
            input: None,
        };
        let result = ingress.respond_permission(response).await;
        if stale {
            result.unwrap();
            assert_eq!(
                bridge.permission_replies(),
                [("command".into(), ClaurstPermissionReply::AllowOnce)]
            );
            assert!(!pending(&ledger));
        } else {
            result.unwrap();
            let policy = ledger
                .current_policy("workspace-a", PolicyScope::ProviderPermissions)
                .unwrap()
                .unwrap();
            assert_eq!(
                policy.policy_id,
                "provider-permissions-v2-ab50b6065e8c3ddbad0696feb67aa85d0d506b225e419ed9593f2d8ef3d2a6ac"
            );
            assert_eq!(policy.allowed_tools, ["Bash"]);
            assert_eq!(policy.revision, 2);
            assert_eq!(
                bridge.permission_replies(),
                [("command".into(), ClaurstPermissionReply::AllowOnce)]
            );
            assert!(!pending(&ledger));
        }
    }
}

#[tokio::test]
async fn a_mode_saved_mid_turn_leaves_the_turn_on_its_launch_policy_until_the_next_launch() {
    use gent_types::{PermissionDecisionResponse, PermissionDecisionResponseKind};
    let (ledger, bridge, mut ingress) =
        prepare_permissions(AgentChatMode::Agent, PermissionMode::AskEveryTime, vec![]).await;
    let mut saved = ledger
        .current_policy("workspace-a", PolicyScope::ProviderPermissions)
        .unwrap()
        .unwrap();
    saved.policy_id = "auto-accept-saved-mid-turn".into();
    saved.revision = 2;
    saved.mode = PermissionMode::AutoAcceptEdits;
    ledger.save_provider_permission_policy(&saved).unwrap();
    let edit = |id: &str| ClaurstDrainBatch {
        facts: vec![],
        permissions: vec![permission(
            id,
            "Edit",
            PermissionCategory::Edit,
            serde_json::json!({"file_path": "/workspace-a/src/app.py"}),
        )],
        checkpoint: Some(checkpoint(0)),
        session_binding: Some(binding()),
        terminal: None,
    };
    bridge.push_batch(edit("edit-1"));
    ingress
        .drain(&binding().source_id, HostEpoch(1))
        .await
        .unwrap();
    assert!(bridge.permission_replies().is_empty());
    let request = ledger
        .pending_permission(
            &AgentChatConversationId("conversation-a".into()),
            &AgentChatRunId("run-a".into()),
        )
        .unwrap()
        .expect("the launch-bound Ask policy still asks during the running turn");
    assert_eq!(request.binding.policy_revision, 1);
    ingress
        .respond_permission(PermissionDecisionResponse {
            binding: request.binding,
            response: PermissionDecisionResponseKind::ApproveOnce,
            input: None,
        })
        .await
        .unwrap();
    bridge.bind_turn_policy(saved.for_conversation(gent_types::Containment::Sandboxed));
    bridge.push_batch(edit("edit-2"));
    ingress
        .drain(&binding().source_id, HostEpoch(1))
        .await
        .unwrap();
    assert_eq!(
        bridge.permission_replies(),
        [
            ("edit-1".to_owned(), ClaurstPermissionReply::AllowOnce),
            ("edit-2".to_owned(), ClaurstPermissionReply::AllowOnce),
        ]
    );
    assert!(!pending(&ledger));
}
