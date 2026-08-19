use gent_ports::{AgentChatPromptLedger, AgentChatWorkspaceLedger, Ledger};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, CapabilitySet, HostEpoch, ReceiptId, WorkspaceRecord,
};

use crate::codex_prompt_lifecycle::{CodexPromptDispatchOutcome, CodexPromptLifecycle};
use crate::codex_prompt_lifecycle_tests::{Resolver, Runner, compatibility, profile};
use crate::public_driver_runtime::PublicDriversRuntime;

#[test]
fn codex_poll_failure_retains_ownership_without_fabricating_terminal_settlement() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = AgentChatConversationId("conversation-a".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id,
                run_id: AgentChatRunId("run-a".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Codex,
                    model: "gpt-5.6".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace-a".into(),
                canonical_path: "/workspace-a".into(),
            },
        )
        .unwrap();
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-a".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            disposition: AgentChatPromptDisposition::Send,
            text: "hello".into(),
        })
        .unwrap();
    crate::readiness_test_support::release(&ledger, &saved);
    let runner = Runner::default();
    let compatibility = compatibility();
    let runtime = PublicDriversRuntime::new(
        profile(&compatibility),
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger.clone(),
        compatibility,
        runner.clone(),
        Resolver,
    )
    .unwrap();
    let mut host = CodexPromptLifecycle::new(runtime, "daemon-a".into());
    assert!(matches!(
        host.dispatch_next(HostEpoch(1)).unwrap(),
        CodexPromptDispatchOutcome::Started { .. }
    ));
    runner.state.lock().unwrap().poll_failure = true;
    let error = host.poll("run-a", HostEpoch(1)).unwrap_err();
    assert!(error.to_string().contains("provider poll unavailable"));
    assert!(!error.to_string().contains("private runner detail"));
    assert!(
        ledger.find_event("codex:run-a:poll:1").unwrap().is_none(),
        "an unproven poll error must not create a terminal fact"
    );
    host.signal_active(gent_drivers::interrupt::ProcessTreeSignal::Interrupt)
        .unwrap();
    assert_eq!(
        runner.state.lock().unwrap().signals,
        [gent_drivers::interrupt::ProcessTreeSignal::Interrupt]
    );
}
