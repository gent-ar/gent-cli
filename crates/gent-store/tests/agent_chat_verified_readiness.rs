use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatPromptLedger, AgentChatSelectionLedger,
    AgentChatWorkspaceLedger, ConversationLedger, Ledger, TranscriptLedger,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, AgentChatSelectionSwitch, Command, ContextPolicy, Event,
    HostEpoch, ProviderPromptReadinessBinding, ProviderPromptReadinessFailureBinding, ReceiptId,
    ReceiptStatus, WorkspaceRecord,
};

fn seeded() -> (SqliteLedger, gent_types::AgentChatPromptSaved) {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = AgentChatConversationId("conversation".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation_id.clone(),
                run_id: AgentChatRunId("run".into()),
                selection: selection(AgentChatProvider::Codex),
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id,
            disposition: AgentChatPromptDisposition::Send,
            text: "continue".into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    (ledger, saved)
}

fn selection(provider: AgentChatProvider) -> AgentChatSelection {
    AgentChatSelection {
        provider,
        model: "model".into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Agent,
    }
}

fn decision(
    saved: &gent_types::AgentChatPromptSaved,
    provider: AgentChatProvider,
) -> (ProviderPromptReadinessBinding, Command, Event) {
    let binding = ProviderPromptReadinessBinding {
        prompt_receipt_id: saved.receipt.receipt_id.clone(),
        conversation_id: AgentChatConversationId(saved.message.conversation_id.clone()),
        run_id: saved.run_id.clone(),
        provider,
    };
    let payload = serde_json::to_value(&binding).unwrap();
    let command = Command {
        receipt_id: ReceiptId("readiness-receipt".into()),
        idempotency_key: "readiness-key".into(),
        host_epoch: HostEpoch(1),
        kind: "agentChatProviderReadiness".into(),
        payload: payload.clone(),
    };
    let terminal = Event {
        cursor: 0,
        event_id: "provider-ready".into(),
        receipt_id: command.receipt_id.clone(),
        host_epoch: command.host_epoch,
        kind: "agentChatProviderReady".into(),
        payload,
    };
    (binding, command, terminal)
}

fn failure_decision(
    saved: &gent_types::AgentChatPromptSaved,
    provider: AgentChatProvider,
    reason: &str,
) -> (ProviderPromptReadinessFailureBinding, Command, Event) {
    let binding = ProviderPromptReadinessFailureBinding {
        prompt_receipt_id: saved.receipt.receipt_id.clone(),
        conversation_id: AgentChatConversationId(saved.message.conversation_id.clone()),
        run_id: saved.run_id.clone(),
        provider,
        reason: reason.into(),
    };
    let payload = serde_json::to_value(&binding).unwrap();
    let command = Command {
        receipt_id: ReceiptId("readiness-failure-receipt".into()),
        idempotency_key: "readiness-failure-key".into(),
        host_epoch: HostEpoch(1),
        kind: "agentChatProviderReadinessFailure".into(),
        payload: payload.clone(),
    };
    let terminal = Event {
        cursor: 0,
        event_id: "provider-readiness-failed".into(),
        receipt_id: command.receipt_id.clone(),
        host_epoch: command.host_epoch,
        kind: "agentChatProviderReadinessFailed".into(),
        payload,
    };
    (binding, command, terminal)
}

#[test]
fn verified_readiness_is_atomic_idempotent_and_only_makes_the_prompt_claimable() {
    let (ledger, saved) = seeded();
    let (binding, command, terminal) = decision(&saved, AgentChatProvider::Codex);
    let receipt = ledger
        .release_verified_agent_chat_prompt_after_readiness(&command, &terminal, &binding)
        .unwrap();
    assert_eq!(receipt.status, ReceiptStatus::Settled);
    assert_eq!(
        ledger
            .release_verified_agent_chat_prompt_after_readiness(&command, &terminal, &binding)
            .unwrap(),
        receipt
    );
    let events = ledger.read_event_page(0, 10).unwrap().events;
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_id == terminal.event_id)
            .count(),
        1
    );
    let claimed = ledger
        .claim_agent_chat_prompt_dispatch("daemon", HostEpoch(1), AgentChatProvider::Codex)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.message, saved.message);
}

#[test]
fn readiness_failure_terminalizes_the_exact_prompt_and_is_idempotent() {
    let (ledger, saved) = seeded();
    let (binding, command, terminal) =
        failure_decision(&saved, AgentChatProvider::Codex, "transport failed");
    let receipt = ledger
        .fail_verified_agent_chat_prompt_after_readiness(&command, &terminal, &binding)
        .unwrap();
    assert_eq!(receipt.status, ReceiptStatus::Settled);
    assert_eq!(
        ledger
            .fail_verified_agent_chat_prompt_after_readiness(&command, &terminal, &binding)
            .unwrap(),
        receipt
    );
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch("daemon", HostEpoch(1), AgentChatProvider::Codex)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        ledger
            .find_turn(&saved.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        gent_types::DurableTurnPhase::Failed
    );
    let transcript = ledger
        .normalized_transcript_page(&AgentChatConversationId("conversation".into()), 0, 10)
        .unwrap();
    assert_eq!(transcript.events.len(), 1);
    assert_eq!(
        transcript.events[0].kind,
        gent_types::NormalizedTranscriptKind::Notice
    );
    assert_eq!(
        transcript.events[0].text,
        "provider readiness failed: transport failed"
    );
    assert_eq!(
        ledger
            .read_event_page(0, 20)
            .unwrap()
            .events
            .iter()
            .filter(|event| event.event_id == terminal.event_id)
            .count(),
        1
    );
}

#[test]
fn readiness_failure_rejects_wrong_provider_and_conflicting_retry() {
    let (ledger, saved) = seeded();
    let (wrong_binding, wrong_command, wrong_terminal) =
        failure_decision(&saved, AgentChatProvider::Claude, "transport failed");
    assert!(
        ledger
            .fail_verified_agent_chat_prompt_after_readiness(
                &wrong_command,
                &wrong_terminal,
                &wrong_binding,
            )
            .is_err()
    );
    let (binding, command, terminal) =
        failure_decision(&saved, AgentChatProvider::Codex, "transport failed");
    ledger
        .fail_verified_agent_chat_prompt_after_readiness(&command, &terminal, &binding)
        .unwrap();
    let conflicting = ProviderPromptReadinessFailureBinding {
        reason: "verification failed".into(),
        ..binding
    };
    let payload = serde_json::to_value(&conflicting).unwrap();
    let conflicting_command = Command {
        payload: payload.clone(),
        ..command
    };
    let conflicting_terminal = Event {
        payload,
        ..terminal
    };
    assert!(
        ledger
            .fail_verified_agent_chat_prompt_after_readiness(
                &conflicting_command,
                &conflicting_terminal,
                &conflicting,
            )
            .is_err()
    );
}

#[test]
fn readiness_release_rejects_stale_epoch_provider_and_terminal_without_exposing_work() {
    let (ledger, saved) = seeded();
    let (binding, mut command, terminal) = decision(&saved, AgentChatProvider::Codex);
    command.host_epoch = HostEpoch(2);
    assert!(
        ledger
            .release_verified_agent_chat_prompt_after_readiness(&command, &terminal, &binding)
            .is_err()
    );
    let (wrong_provider, command, terminal) = decision(&saved, AgentChatProvider::Claude);
    assert!(
        ledger
            .release_verified_agent_chat_prompt_after_readiness(
                &command,
                &terminal,
                &wrong_provider
            )
            .is_err()
    );
    let (binding, command, mut terminal) = decision(&saved, AgentChatProvider::Codex);
    terminal.kind = "not-ready".into();
    assert!(
        ledger
            .release_verified_agent_chat_prompt_after_readiness(&command, &terminal, &binding)
            .is_err()
    );
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch("daemon", HostEpoch(1), AgentChatProvider::Codex)
            .unwrap()
            .is_none()
    );
}

#[test]
fn readiness_event_conflict_rolls_back_its_receipt_and_held_prompt_release() {
    let (ledger, saved) = seeded();
    let (binding, command, terminal) = decision(&saved, AgentChatProvider::Codex);
    ledger
        .append_event(&Event {
            cursor: 0,
            event_id: terminal.event_id.clone(),
            receipt_id: saved.receipt.receipt_id.clone(),
            host_epoch: HostEpoch(1),
            kind: "existing".into(),
            payload: serde_json::Value::Null,
        })
        .unwrap();
    assert!(
        ledger
            .release_verified_agent_chat_prompt_after_readiness(&command, &terminal, &binding)
            .is_err()
    );
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch("daemon", HostEpoch(1), AgentChatProvider::Codex)
            .unwrap()
            .is_none()
    );
    let mut retry_terminal = terminal;
    retry_terminal.event_id = "provider-ready-retry".into();
    assert_eq!(
        ledger
            .release_verified_agent_chat_prompt_after_readiness(
                &command,
                &retry_terminal,
                &binding,
            )
            .unwrap()
            .status,
        ReceiptStatus::Settled
    );
}

#[test]
fn a_current_run_switch_fences_out_an_older_verified_readiness_decision() {
    let (ledger, saved) = seeded();
    ledger
        .switch_agent_chat_selection(&AgentChatSelectionSwitch {
            receipt_id: ReceiptId("switch-receipt".into()),
            idempotency_key: "switch-key".into(),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId(saved.message.conversation_id.clone()),
            parent_run_id: saved.run_id.clone(),
            run_id: AgentChatRunId("new-run".into()),
            selection: selection(AgentChatProvider::Claude),
            context_policy: ContextPolicy::Preserve,
        })
        .unwrap();
    let (binding, command, terminal) = decision(&saved, AgentChatProvider::Codex);
    assert!(
        ledger
            .release_verified_agent_chat_prompt_after_readiness(&command, &terminal, &binding)
            .is_err()
    );
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch("daemon", HostEpoch(1), AgentChatProvider::Codex)
            .unwrap()
            .is_none()
    );
}
