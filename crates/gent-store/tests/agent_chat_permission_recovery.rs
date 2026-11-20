use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatPromptLedger, AgentChatWorkspaceLedger,
    ConversationActivityLedger, Ledger, PendingPermissionLedger,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatDecisionId, AgentChatEffort,
    AgentChatMode, AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider,
    AgentChatRequestId, AgentChatRunId, AgentChatSelection, ConversationActivityFact, HostEpoch,
    PermissionCategory, PermissionDecisionBinding, PermissionDecisionRequest, PermissionRequest,
    PermissionRequestDigest, ReceiptId, WorkspaceRecord,
};

fn conversation(ledger: &SqliteLedger, name: &str) -> (AgentChatConversationId, AgentChatRunId) {
    let conversation_id = AgentChatConversationId(format!("conversation-{name}"));
    let run_id = AgentChatRunId(format!("run-{name}"));
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId(format!("create-{name}")),
                idempotency_key: format!("create-key-{name}"),
                host_epoch: HostEpoch(1),
                conversation_id: conversation_id.clone(),
                run_id: run_id.clone(),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Codex,
                    model: "gpt-5.6".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(format!("prompt-{name}")),
            receipt_id: ReceiptId(format!("prompt-receipt-{name}")),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            disposition: AgentChatPromptDisposition::Send,
            text: format!("prompt {name}"),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    ledger
        .save_pending_permission(&PermissionDecisionRequest {
            binding: PermissionDecisionBinding {
                decision_id: AgentChatDecisionId("0".into()),
                request_idempotency_key: format!("codex:{}:0", run_id.0),
                conversation_id: conversation_id.clone(),
                run_id: run_id.clone(),
                turn_id: saved.message.turn_id,
                policy_id: "policy-1".into(),
                policy_revision: 1,
                host_epoch: HostEpoch(1),
                request_digest_sha256: PermissionRequestDigest("a".repeat(64)),
            },
            request: PermissionRequest {
                tool_name: "Command".into(),
                category: PermissionCategory::Command,
                input: None,
                child_id: None,
            },
        })
        .unwrap();
    (conversation_id, run_id)
}

fn settled(
    ledger: &SqliteLedger,
    conversation: &AgentChatConversationId,
    run: &AgentChatRunId,
) -> bool {
    ledger
        .read_conversation_activity_page(&conversation.0, &run.0, 0, gent_ports::MAX_CONVERSATION_ACTIVITY_PAGE_FACTS)
        .unwrap()
        .facts
        .iter()
        .any(|fact| matches!(fact, ConversationActivityFact::DecisionSettled { scope, .. } if scope.host_epoch == HostEpoch(2)))
}

#[test]
fn startup_recovery_settles_every_stale_permission_it_can_and_never_wedges_on_a_bad_row() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    let (first, first_run) = conversation(&ledger, "a");
    let (duplicate, duplicate_run) = conversation(&ledger, "b");
    let (malformed, malformed_run) = conversation(&ledger, "c");
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute(
        "UPDATE pending_provider_permissions SET request_json = (SELECT request_json FROM pending_provider_permissions WHERE run_id = ?1) WHERE run_id = ?2",
        [&first_run.0, &duplicate_run.0],
    )
    .unwrap();
    raw.execute(
        "UPDATE pending_provider_permissions SET request_json = '{' WHERE run_id = ?1",
        [&malformed_run.0],
    )
    .unwrap();
    drop(raw);
    ledger.close_ingress(HostEpoch(1)).unwrap();
    ledger.fence_and_open(HostEpoch(1)).unwrap();

    ledger
        .recover_agent_chat_prompt_dispatches(HostEpoch(2))
        .unwrap();

    for (conversation, run) in [
        (&first, &first_run),
        (&duplicate, &duplicate_run),
        (&malformed, &malformed_run),
    ] {
        assert!(
            ledger
                .pending_permission(conversation, run)
                .unwrap()
                .is_none()
        );
    }
    assert!(settled(&ledger, &first, &first_run));
    assert!(!settled(&ledger, &duplicate, &duplicate_run));
    assert!(!settled(&ledger, &malformed, &malformed_run));
}
