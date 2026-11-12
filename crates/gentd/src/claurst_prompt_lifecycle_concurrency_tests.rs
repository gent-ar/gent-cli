use gent_ports::{
    AgentChatPromptLedger, AgentChatWorkspaceLedger, ClaurstCheckpoint, ClaurstDrainBatch,
    ClaurstPermissionReply, ClaurstPermissionRequest, ClaurstSessionBinding, ClaurstSourceId,
    ClaurstTerminal, ConversationLedger, PendingPermissionLedger, PolicyLedger,
};
use gent_store::SqliteLedger;
use gent_testkit::FakePrivateClaurstBridge;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatProvider,
    AgentChatRequestId, AgentChatRunId, AgentChatSelection, DurableTurnPhase, HostEpoch,
    PermissionCategory, PermissionDecisionResponse, PermissionDecisionResponseKind, PermissionMode,
    PolicyRecord, PolicyScope, ReceiptId, WorkspaceRecord,
};
use sha2::{Digest, Sha256};

use super::{AsyncOrdinaryLifecycleHost, ClaurstPromptLifecycle};
use crate::claurst_runtime_factory::CLAURST_CONVERSATION_CAPACITY;

fn conversation(ledger: &SqliteLedger, name: &str) {
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId(format!("conversation-receipt-{name}")),
                idempotency_key: format!("conversation-key-{name}"),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId(format!("conversation-{name}")),
                run_id: AgentChatRunId(format!("run-{name}")),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claurst,
                    model: "qwen3-1-7b-q4-k-m".into(),
                    effort: AgentChatEffort::Low,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace-a".into(),
                canonical_path: "/workspace-a".into(),
            },
        )
        .unwrap();
}

fn prompt(
    ledger: &SqliteLedger,
    bridge: &FakePrivateClaurstBridge,
    name: &str,
) -> (AgentChatPromptSaved, ClaurstSessionBinding) {
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(format!("request-{name}")),
            receipt_id: ReceiptId(format!("receipt-{name}")),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId(format!("conversation-{name}")),
            disposition: AgentChatPromptDisposition::Send,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: format!("write the file for {name}"),
        })
        .unwrap();
    crate::readiness_test_support::release(ledger, &saved);
    let material = format!(
        "{}\0{}\0{}",
        saved.run_id.0, saved.message.turn_id, saved.message.message_id
    );
    let binding = ClaurstSessionBinding {
        run_id: saved.run_id.0.clone(),
        source_id: ClaurstSourceId(format!("gent-{}", hex::encode(Sha256::digest(material)))),
        opaque_session_id: format!("acp-session-{name}"),
    };
    bridge.push_start_binding(binding.clone());
    (saved, binding)
}

fn batch(binding: &ClaurstSessionBinding) -> ClaurstDrainBatch {
    ClaurstDrainBatch {
        facts: vec![],
        permissions: vec![],
        checkpoint: Some(ClaurstCheckpoint {
            run_id: binding.run_id.clone(),
            source_id: binding.source_id.clone(),
            cursor: 0,
            state_digest_sha256: "d".repeat(64),
        }),
        session_binding: Some(binding.clone()),
        terminal: None,
    }
}

fn completed(binding: &ClaurstSessionBinding) -> ClaurstDrainBatch {
    ClaurstDrainBatch {
        terminal: Some(ClaurstTerminal::Completed),
        ..batch(binding)
    }
}

fn phase(ledger: &SqliteLedger, saved: &AgentChatPromptSaved) -> DurableTurnPhase {
    ledger
        .find_turn(&saved.message.turn_id)
        .unwrap()
        .unwrap()
        .phase
}

fn waiting_permission(ledger: &SqliteLedger) -> Option<gent_types::PermissionDecisionRequest> {
    ledger
        .pending_permission(
            &AgentChatConversationId("conversation-a".into()),
            &AgentChatRunId("run-a".into()),
        )
        .unwrap()
}

async fn drive(
    lifecycle: &mut ClaurstPromptLifecycle<SqliteLedger, FakePrivateClaurstBridge>,
    bridge: &FakePrivateClaurstBridge,
    running: &[&ClaurstSessionBinding],
) {
    for binding in running {
        bridge.push_batch(batch(binding));
    }
    lifecycle.drive_once().await.unwrap();
}

async fn recovered(
    ledger: &SqliteLedger,
    bridge: &FakePrivateClaurstBridge,
) -> ClaurstPromptLifecycle<SqliteLedger, FakePrivateClaurstBridge> {
    let mut lifecycle = ClaurstPromptLifecycle::new(
        ledger.clone(),
        bridge.clone(),
        "gentd-1".into(),
        HostEpoch(1),
    );
    lifecycle.activate_recovery().await.unwrap();
    lifecycle
}

#[tokio::test]
async fn a_conversation_waiting_on_a_permission_never_blocks_another_conversation() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let bridge = FakePrivateClaurstBridge::default();
    let policy = PolicyRecord {
        policy_id: "policy-a".into(),
        workspace_id: "workspace-a".into(),
        scope: PolicyScope::ProviderPermissions,
        revision: 1,
        mode: PermissionMode::AskEveryTime,
        containment: gent_types::Containment::Unsandboxed,
        allowed_tools: vec![],
        allowed_categories: vec![],
    };
    conversation(&ledger, "a");
    conversation(&ledger, "b");
    ledger.save_policy(&policy).unwrap();
    bridge.bind_turn_policy(policy);
    let (waiting, a) = prompt(&ledger, &bridge, "a");
    let mut lifecycle = recovered(&ledger, &bridge).await;
    drive(&mut lifecycle, &bridge, &[]).await;
    bridge.push_batch(ClaurstDrainBatch {
        permissions: vec![ClaurstPermissionRequest {
            request_id: "write-a".into(),
            tool_use_id: "tool-write-a".into(),
            tool_name: "Write".into(),
            category: PermissionCategory::Edit,
            input: Some(serde_json::json!({"file_path":"/workspace-a/a.txt","content":"a"})),
        }],
        ..batch(&a)
    });
    drive(&mut lifecycle, &bridge, &[]).await;
    let pending =
        waiting_permission(&ledger).expect("conversation A waits on its write permission");

    let (other, b) = prompt(&ledger, &bridge, "b");
    drive(&mut lifecycle, &bridge, &[&a]).await;
    assert_eq!(bridge.starts().len(), 2, "B starts while A waits");
    bridge.push_batch(completed(&b));
    drive(&mut lifecycle, &bridge, &[&a]).await;
    assert_eq!(phase(&ledger, &other), DurableTurnPhase::Completed);
    assert_eq!(phase(&ledger, &waiting), DurableTurnPhase::Active);
    assert_eq!(waiting_permission(&ledger), Some(pending.clone()));

    lifecycle
        .respond_permission(PermissionDecisionResponse {
            binding: pending.binding,
            response: PermissionDecisionResponseKind::ApproveOnce,
            input: None,
        })
        .await
        .unwrap();
    assert_eq!(
        bridge.permission_replies(),
        [("write-a".into(), ClaurstPermissionReply::AllowOnce)]
    );
    bridge.push_batch(completed(&a));
    drive(&mut lifecycle, &bridge, &[]).await;
    assert_eq!(phase(&ledger, &waiting), DurableTurnPhase::Completed);
    assert_eq!(waiting_permission(&ledger), None);
    assert!(bridge.cancellations().is_empty());
}

#[tokio::test]
async fn concurrent_conversations_are_bounded_and_the_next_starts_when_one_settles() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let bridge = FakePrivateClaurstBridge::default();
    let names = ["a", "b", "c", "d"];
    assert!(names.len() > CLAURST_CONVERSATION_CAPACITY);
    let prompts = names
        .iter()
        .map(|name| {
            conversation(&ledger, name);
            prompt(&ledger, &bridge, name)
        })
        .collect::<Vec<_>>();
    let bindings = prompts
        .iter()
        .map(|(_, binding)| binding)
        .collect::<Vec<_>>();
    let mut lifecycle = recovered(&ledger, &bridge).await;
    for started in 0..=names.len() {
        let running = started.min(CLAURST_CONVERSATION_CAPACITY);
        drive(&mut lifecycle, &bridge, &bindings[..running]).await;
    }
    assert_eq!(bridge.starts().len(), CLAURST_CONVERSATION_CAPACITY);
    let (first, first_binding) = &prompts[0];
    bridge.push_batch(completed(first_binding));
    drive(
        &mut lifecycle,
        &bridge,
        &bindings[1..CLAURST_CONVERSATION_CAPACITY],
    )
    .await;
    assert_eq!(phase(&ledger, first), DurableTurnPhase::Completed);
    assert_eq!(bridge.starts().len(), names.len());
}

#[tokio::test]
async fn one_conversation_still_runs_one_prompt_at_a_time() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let bridge = FakePrivateClaurstBridge::default();
    conversation(&ledger, "a");
    let (running, a) = prompt(&ledger, &bridge, "a");
    let mut lifecycle = recovered(&ledger, &bridge).await;
    drive(&mut lifecycle, &bridge, &[]).await;
    let queued = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("request-a-2".into()),
            receipt_id: ReceiptId("receipt-a-2".into()),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            disposition: AgentChatPromptDisposition::Queue,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: "next".into(),
        })
        .unwrap();
    crate::readiness_test_support::release(&ledger, &queued);
    drive(&mut lifecycle, &bridge, &[&a]).await;
    drive(&mut lifecycle, &bridge, &[&a]).await;
    assert_eq!(
        bridge.starts().len(),
        1,
        "a running turn holds its conversation"
    );
    bridge.push_batch(completed(&a));
    drive(&mut lifecycle, &bridge, &[]).await;
    assert_eq!(phase(&ledger, &running), DurableTurnPhase::Completed);
    assert_eq!(bridge.starts().len(), 2, "the queued prompt runs next");
}
