use super::*;
use gent_ports::{AgentChatWorkspaceLedger, ConversationLedger, PolicyLedger};
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
fn unattended_modes_save_while_work_in_the_workspace_is_active_or_waiting() {
    let (ledger, mut latest) = fixture();
    for (phase, mode) in [
        (DurableTurnPhase::Active, PermissionMode::Autonomous),
        (DurableTurnPhase::WaitingPermission, PermissionMode::Bypass),
        (
            DurableTurnPhase::WaitingQuestion,
            PermissionMode::AskEveryTime,
        ),
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
        let mut next = latest.clone();
        next.policy_id = format!("next-{mode:?}");
        next.revision += 1;
        next.mode = mode;
        ledger.save_provider_permission_policy(&next).unwrap();
        assert_eq!(
            ledger
                .current_policy("workspace", PolicyScope::ProviderPermissions)
                .unwrap(),
            Some(next.clone())
        );
        latest = next;
    }
}

#[test]
fn exact_revalidation_is_idempotent_and_the_next_revision_becomes_current() {
    let (ledger, mut broad) = fixture();
    broad.policy_id = "broad".into();
    broad.revision += 1;
    broad.mode = PermissionMode::Bypass;
    ledger.save_provider_permission_policy(&broad).unwrap();
    ledger.save_provider_permission_policy(&broad).unwrap();
    let mut changed = broad.clone();
    changed.policy_id = "changed".into();
    changed.revision += 1;
    changed.mode = PermissionMode::AskEveryTime;
    ledger.save_provider_permission_policy(&changed).unwrap();
    assert_eq!(
        ledger
            .current_policy("workspace", PolicyScope::ProviderPermissions)
            .unwrap(),
        Some(changed)
    );
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
