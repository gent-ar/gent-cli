use super::*;
use gent_ports::{AgentChatWorkspaceLedger, ConversationLedger, PolicyLedger, WorkspaceLedger};
use gent_types::*;

fn fixture() -> (SqliteLedger, PolicyRecord) {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create".into()),
                idempotency_key: "create".into(),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation".into()),
                run_id: AgentChatRunId("run".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "model".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    let policy = PolicyRecord {
        policy_id: "policy".into(),
        workspace_id: "workspace".into(),
        scope: PolicyScope::ProviderPermissions,
        revision: 1,
        mode: PermissionMode::AskEveryTime,
        containment: gent_types::Containment::Sandboxed,
        allowed_tools: vec![],
        allowed_categories: vec![],
    };
    ledger.save_policy(&policy).unwrap();
    ledger
        .create_turn(&TurnRecord {
            turn_id: "turn".into(),
            conversation_id: "conversation".into(),
            run_id: "run".into(),
            sequence: 1,
            phase: DurableTurnPhase::Active,
        })
        .unwrap();
    (ledger, policy)
}

#[test]
fn durable_commit_rechecks_active_work_even_if_an_earlier_preflight_was_idle() {
    let (ledger, current) = fixture();
    let mut next = current.clone();
    next.policy_id = "next".into();
    next.revision += 1;
    next.mode = PermissionMode::Autonomous;
    for phase in [
        DurableTurnPhase::Active,
        DurableTurnPhase::WaitingPermission,
        DurableTurnPhase::WaitingQuestion,
    ] {
        if phase != DurableTurnPhase::Active {
            ledger
                .replace_turn_phase(
                    "turn",
                    if phase == DurableTurnPhase::WaitingPermission {
                        DurableTurnPhase::Active
                    } else {
                        DurableTurnPhase::WaitingPermission
                    },
                    phase,
                )
                .unwrap();
        }
        assert!(ledger.save_provider_permission_policy(&next).is_err());
        assert_eq!(
            ledger
                .current_policy("workspace", PolicyScope::ProviderPermissions)
                .unwrap(),
            Some(current.clone())
        );
    }
    ledger
        .replace_turn_phase(
            "turn",
            DurableTurnPhase::WaitingQuestion,
            DurableTurnPhase::Completed,
        )
        .unwrap();
    ledger.save_provider_permission_policy(&next).unwrap();
    assert_eq!(
        ledger
            .current_policy("workspace", PolicyScope::ProviderPermissions)
            .unwrap(),
        Some(next)
    );
}

#[test]
fn exact_broad_revalidation_is_idempotent_while_a_new_binding_revision_is_rejected() {
    let (ledger, mut broad) = fixture();
    broad.policy_id = "broad".into();
    broad.revision += 1;
    broad.mode = PermissionMode::Bypass;
    // Raw persistence intentionally seeds a legacy/live-bound fixture; product commits use the guarded port.
    ledger.save_policy(&broad).unwrap();
    ledger.save_provider_permission_policy(&broad).unwrap();
    let mut changed = broad.clone();
    changed.policy_id = "changed".into();
    changed.revision += 1;
    assert!(ledger.save_provider_permission_policy(&changed).is_err());
    changed.mode = PermissionMode::AskEveryTime;
    assert!(ledger.save_provider_permission_policy(&changed).is_err());
    assert_eq!(
        ledger
            .current_policy("workspace", PolicyScope::ProviderPermissions)
            .unwrap(),
        Some(broad)
    );
}

#[test]
fn unrelated_workspace_activity_does_not_block_a_policy_commit() {
    let (ledger, mut policy) = fixture();
    ledger
        .create_workspace(&WorkspaceRecord {
            workspace_id: "other".into(),
            canonical_path: "/other".into(),
        })
        .unwrap();
    policy.policy_id = "other".into();
    policy.workspace_id = "other".into();
    policy.mode = PermissionMode::Autonomous;
    ledger.save_provider_permission_policy(&policy).unwrap();
}

#[test]
fn replaying_an_older_immutable_record_cannot_acknowledge_it_as_current() {
    let (ledger, current) = fixture();
    let mut next = current.clone();
    next.policy_id = "next".into();
    next.revision += 1;
    next.mode = PermissionMode::AutoAcceptEdits;
    ledger.save_provider_permission_policy(&next).unwrap();
    assert!(
        ledger
            .save_provider_permission_policy(&current)
            .unwrap_err()
            .to_string()
            .contains("policy changed")
    );
    assert_eq!(
        ledger
            .current_policy("workspace", PolicyScope::ProviderPermissions)
            .unwrap(),
        Some(next)
    );
}
