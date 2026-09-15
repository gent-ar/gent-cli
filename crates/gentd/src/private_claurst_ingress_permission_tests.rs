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

async fn drain_permissions(
    mode: AgentChatMode,
    policy: PermissionMode,
    permissions: Vec<ClaurstPermissionRequest>,
) -> (SqliteLedger, FakePrivateClaurstBridge) {
    let ledger = prepared_ledger_in_mode(mode);
    ledger
        .save_policy(&PolicyRecord {
            policy_id: "policy-a".into(),
            workspace_id: "workspace-a".into(),
            scope: PolicyScope::ProviderPermissions,
            revision: 1,
            mode: policy,
            allowed_tools: vec![],
            allowed_categories: vec![],
        })
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
        PermissionMode::Autonomous,
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
async fn an_autonomous_command_waits_for_the_user_without_an_os_sandbox() {
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
