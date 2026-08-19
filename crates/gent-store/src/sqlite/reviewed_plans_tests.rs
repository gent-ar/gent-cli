use gent_ports::{
    AgentChatPromptLedger, AgentChatRunContextReader, AgentChatWorkspaceLedger, PolicyLedger,
    ReviewedPlanLedger, WorkspaceLedger,
};
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunContextOrigin, AgentChatRunId, AgentChatSelection, ContextPolicy, HostEpoch,
    PermissionMode, PlanAction, PlanActionKind, PlanArtifact, PlanRevision, PlanStatus,
    PolicyRecord, PolicyScope, ReceiptId, ReviewedPlanId, StartImplementationRequest,
    WorkspaceRecord,
};

use super::SqliteLedger;

fn selection() -> AgentChatSelection {
    AgentChatSelection {
        provider: AgentChatProvider::Codex,
        model: "gpt-5.6".into(),
        effort: AgentChatEffort::High,
        mode: AgentChatMode::Plan,
    }
}

fn seeded() -> (SqliteLedger, PlanArtifact, StartImplementationRequest) {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_workspace(&WorkspaceRecord {
            workspace_id: "workspace-1".into(),
            canonical_path: "/workspace".into(),
        })
        .unwrap();
    ledger
        .save_policy(&PolicyRecord {
            policy_id: "policy-1".into(),
            workspace_id: "workspace-1".into(),
            scope: PolicyScope::ProviderPermissions,
            revision: 1,
            mode: PermissionMode::Plan,
            allowed_tools: vec![],
            allowed_categories: vec![],
        })
        .unwrap();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create-receipt".into()),
                idempotency_key: "create-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation-1".into()),
                run_id: AgentChatRunId("run-1".into()),
                selection: selection(),
            },
            &WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    let prompt = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-1".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation-1".into()),
            disposition: AgentChatPromptDisposition::Send,
            text: "Make the change".into(),
        })
        .unwrap();
    let plan = PlanArtifact {
        plan_id: ReviewedPlanId("plan-1".into()),
        conversation_id: AgentChatConversationId("conversation-1".into()),
        source_run_id: AgentChatRunId("run-1".into()),
        source_turn_id: prompt.message.turn_id,
        revision: PlanRevision(1),
        content_digest_sha256: "a".repeat(64),
        status: PlanStatus::ReadyForReview,
        actions: vec![PlanAction {
            action_id: "edit-1".into(),
            kind: PlanActionKind::Edit,
            summary: "Update one file".into(),
        }],
        risks: vec![],
        diffs: vec![],
        permission_preview: vec![],
    };
    let request = StartImplementationRequest {
        request_id: AgentChatRequestId("approve-1".into()),
        receipt_id: ReceiptId("approve-receipt".into()),
        idempotency_key: "approve-key".into(),
        host_epoch: HostEpoch(1),
        policy_workspace_id: "workspace-1".into(),
        policy_revision: 1,
        conversation_id: plan.conversation_id.clone(),
        plan_id: plan.plan_id.clone(),
        plan_revision: plan.revision,
        plan_content_digest_sha256: plan.content_digest_sha256.clone(),
        parent_run_id: plan.source_run_id.clone(),
        selection: selection(),
        context_policy: ContextPolicy::Clear,
    };
    (ledger, plan, request)
}

#[test]
fn trusted_plan_approval_is_atomic_retry_safe_and_clear_has_no_session_boundary() {
    let (ledger, plan, request) = seeded();
    ledger.save_trusted_plan(&plan).unwrap();
    let first = ledger.approve_reviewed_plan(&request).unwrap();
    let retry = ledger.approve_reviewed_plan(&request).unwrap();
    assert_eq!(first, retry);
    assert_eq!(first.context_through_ordinal, 0);
    let context = ledger
        .read_agent_chat_run_context(&first.conversation_id, &first.implementation_run_id)
        .unwrap();
    assert_eq!(context.origin, AgentChatRunContextOrigin::ReviewedPlan);
    assert_eq!(context.context_policy, ContextPolicy::Clear);
    assert_eq!(context.context_through_ordinal, 0);
    assert_eq!(first.receipt.host_epoch, HostEpoch(1));
    let bindings: u64 = ledger
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM run_session_bindings WHERE run_id = ?1",
            [&first.implementation_run_id.0],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(bindings, 0);
    assert_eq!(
        ledger
            .reviewed_plan("conversation-1", &ReviewedPlanId("plan-1".into()))
            .unwrap()
            .unwrap()
            .status,
        PlanStatus::Approved
    );
}

#[test]
fn preserve_context_freezes_the_existing_history_ordinal() {
    let (ledger, plan, mut request) = seeded();
    ledger.save_trusted_plan(&plan).unwrap();
    request.context_policy = ContextPolicy::Preserve;
    let expected: u64 = ledger
        .lock()
        .unwrap()
        .query_row(
            "SELECT COALESCE(MAX(ordinal), 0) FROM conversation_message_ordinals WHERE conversation_id = ?1",
            ["conversation-1"],
            |row| row.get(0),
        )
        .unwrap();

    let result = ledger.approve_reviewed_plan(&request).unwrap();

    assert!(expected > 0);
    assert_eq!(result.context_policy, ContextPolicy::Preserve);
    assert_eq!(result.context_through_ordinal, expected);
    let context = ledger
        .read_agent_chat_run_context(&result.conversation_id, &result.implementation_run_id)
        .unwrap();
    assert_eq!(context.context_policy, ContextPolicy::Preserve);
    assert_eq!(context.context_through_ordinal, expected);
}

#[test]
fn exact_revision_and_current_policy_are_rechecked_before_any_child_exists() {
    let (ledger, plan, mut request) = seeded();
    ledger.save_trusted_plan(&plan).unwrap();
    request.plan_content_digest_sha256 = "b".repeat(64);
    assert!(ledger.approve_reviewed_plan(&request).is_err());
    request.plan_content_digest_sha256 = plan.content_digest_sha256.clone();
    request.policy_revision = 2;
    assert!(ledger.approve_reviewed_plan(&request).is_err());
    ledger
        .reject_reviewed_plan(&plan.plan_id, plan.revision, &plan.content_digest_sha256)
        .unwrap();
    assert!(ledger.approve_reviewed_plan(&request).is_err());
}
