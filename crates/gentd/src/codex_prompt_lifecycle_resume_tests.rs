use gent_ports::{
    AgentChatPromptLedger, AgentChatWorkspaceLedger, Ledger, RunLease, RunSessionBinding,
};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, CapabilitySet, HostEpoch, ReceiptId, WorkspaceRecord,
};

use crate::codex_prompt_lifecycle::{CodexPromptDispatchOutcome, CodexPromptLifecycle};
use crate::codex_prompt_lifecycle_tests::{Resolver, Runner, compatibility, lock, profile};
use crate::public_driver_runtime::PublicDriversRuntime;

#[test]
fn next_prompt_resumes_the_daemon_owned_codex_session_after_process_loss() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = AgentChatConversationId("conversation-a".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation_id.clone(),
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
    ledger
        .activate_existing_run_start(
            &lock(),
            &RunLease {
                run_id: "run-a".into(),
                coordinator_id: "daemon-a".into(),
                host_epoch: HostEpoch(1),
            },
        )
        .unwrap();
    ledger
        .save_run_session_binding(&RunSessionBinding {
            run_id: "run-a".into(),
            provider_session_id: "daemon-owned-thread".into(),
        })
        .unwrap();
    ledger.close_ingress(HostEpoch(1)).unwrap();
    ledger.fence_and_open(HostEpoch(1)).unwrap();
    ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-a".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            host_epoch: HostEpoch(2),
            conversation_id,
            disposition: AgentChatPromptDisposition::Send,
            text: "resume me".into(),
        })
        .unwrap();
    let runner = Runner::default();
    let compatibility = compatibility();
    let runtime = PublicDriversRuntime::new(
        profile(&compatibility),
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger,
        compatibility,
        runner.clone(),
        Resolver,
    )
    .unwrap();
    let mut host = CodexPromptLifecycle::new(runtime, "daemon-b".into(), None);
    assert!(matches!(
        host.dispatch_next(HostEpoch(2)).unwrap(),
        CodexPromptDispatchOutcome::Started { .. }
    ));
    let state = runner.state.lock().unwrap();
    assert_eq!(state.starts, 0);
    assert_eq!(state.resumes, 1);
}
