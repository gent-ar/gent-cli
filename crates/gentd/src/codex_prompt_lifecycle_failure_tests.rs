use gent_ports::{AgentChatLedger, AgentChatPromptDispatchLedger, AgentChatPromptLedger, Ledger};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, CapabilitySet, HostEpoch, ReceiptId,
};

use crate::codex_prompt_lifecycle::{CodexPromptDispatchOutcome, CodexPromptLifecycle};
use crate::codex_prompt_lifecycle_tests::{Resolver, Runner, compatibility, profile};
use crate::public_driver_runtime::PublicDriversRuntime;

#[test]
fn codex_poll_failure_settles_without_persisting_private_runner_detail() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = AgentChatConversationId("conversation-a".into());
    ledger
        .create_agent_chat_conversation(&AgentChatConversationCreate {
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
        })
        .unwrap();
    ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-a".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            disposition: AgentChatPromptDisposition::Send,
            text: "hello".into(),
        })
        .unwrap();
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
    let mut host = CodexPromptLifecycle::new(runtime, "daemon-a".into(), None);
    assert!(matches!(
        host.dispatch_next(HostEpoch(1)).unwrap(),
        CodexPromptDispatchOutcome::Started { .. }
    ));
    runner.state.lock().unwrap().poll_failure = true;
    assert!(host.poll("run-a", HostEpoch(1)).unwrap().unwrap().exited);
    let event = ledger.find_event("codex:run-a:poll:1").unwrap().unwrap();
    assert!(!event.payload.to_string().contains("private runner detail"));
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
            .unwrap()
            .is_none()
    );
}
