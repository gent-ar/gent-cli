use gent_ports::{
    AgentChatSelectionLedger, PrivateProviderPromptProvisionLedger, ProvisionedProviderLockLedger,
};
use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRunId,
    AgentChatSelection, ContextPolicy, HostEpoch, ReceiptId, ReceiptStatus,
};

pub(super) use super::test_support::{
    dispatch_state, installation, provision, receipt_status, seeded, settle, terminal,
};

#[test]
fn verified_install_and_exact_prompt_release_commit_together() {
    let (ledger, saved) = seeded();
    let (binding, command, receipt) = provision(&ledger, &saved);
    assert_eq!(
        dispatch_state(&ledger, &saved.message.message_id),
        "provisioning"
    );
    settle(&ledger, &binding, &command, &receipt).unwrap();
    assert_eq!(
        dispatch_state(&ledger, &saved.message.message_id),
        "pending"
    );
    assert_eq!(
        ledger
            .find_provisioned_provider_installation("codex")
            .unwrap(),
        Some(installation(&command, &binding))
    );
    assert_eq!(
        settle(&ledger, &binding, &command, &receipt)
            .unwrap()
            .status,
        ReceiptStatus::Settled
    );
}

#[test]
fn event_conflict_rolls_back_lock_receipt_and_prompt_release() {
    let (ledger, saved) = seeded();
    let (binding, command, receipt) = provision(&ledger, &saved);
    let mut terminal = terminal(&receipt);
    terminal.event_id = "provision-accepted".into();
    assert!(
        ledger
            .settle_verified_provider_prompt_provision(
                &command,
                &receipt,
                &installation(&command, &binding),
                &terminal,
                &binding,
            )
            .is_err()
    );
    assert_eq!(
        dispatch_state(&ledger, &saved.message.message_id),
        "provisioning"
    );
    assert!(
        ledger
            .find_provisioned_provider_installation("codex")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        receipt_status(&ledger, &receipt.idempotency_key),
        "accepted"
    );
}

#[test]
fn stale_or_mismatched_prompt_binding_cannot_release_a_prompt() {
    let (ledger, saved) = seeded();
    let (mut binding, command, receipt) = provision(&ledger, &saved);
    binding.prompt.run_id = AgentChatRunId("another-run".into());
    assert!(
        ledger
            .settle_verified_provider_prompt_provision(
                &command,
                &receipt,
                &installation(&command, &binding),
                &terminal(&receipt),
                &binding,
            )
            .is_err()
    );
    assert_eq!(
        dispatch_state(&ledger, &saved.message.message_id),
        "provisioning"
    );
    assert!(
        ledger
            .find_provisioned_provider_installation("codex")
            .unwrap()
            .is_none()
    );
}

#[test]
fn provision_admission_blocks_a_selection_switch_until_its_terminal_settlement() {
    let (ledger, saved) = seeded();
    let (_binding, _command, _receipt) = provision(&ledger, &saved);
    assert!(
        ledger
            .switch_agent_chat_selection(&gent_types::AgentChatSelectionSwitch {
                receipt_id: ReceiptId("switch-receipt".into()),
                idempotency_key: "switch-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation".into()),
                parent_run_id: saved.run_id.clone(),
                run_id: AgentChatRunId("next-run".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "sonnet".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Plan,
                },
                context_policy: ContextPolicy::Preserve,
            })
            .is_err()
    );
    assert_eq!(
        dispatch_state(&ledger, &saved.message.message_id),
        "provisioning"
    );
}
