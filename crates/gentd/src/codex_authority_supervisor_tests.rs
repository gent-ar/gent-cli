use crate::approved_codex_host::ApprovedCodexHost;
use crate::codex_authority_supervisor::{
    PrivateCodexShutdown, PrivateCodexSupervisor, PrivateCodexSupervisorState, PrivateCodexWake,
};
use crate::codex_prompt_lifecycle::CodexPromptDispatchOutcome;
use crate::codex_prompt_lifecycle_tests::{Resolver, Runner, compatibility, profile, selection};
use crate::public_driver_runtime::PublicDriversRuntime;
use gent_ports::{AgentChatLedger, AgentChatPromptLedger, Ledger};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatPromptCreate,
    AgentChatPromptDisposition, AgentChatRequestId, AgentChatRunId, CapabilitySet, HostEpoch,
    ReceiptId,
};

#[test]
fn first_wake_recovers_then_runs_one_bounded_tick() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let mut supervisor = PrivateCodexSupervisor::new(host(&ledger, Runner::default()));
    assert_eq!(
        supervisor.state(),
        PrivateCodexSupervisorState::AwaitingRecovery
    );
    assert!(matches!(
        supervisor.wake().unwrap(),
        PrivateCodexWake::Tick(tick) if tick.dispatch == Some(CodexPromptDispatchOutcome::Empty)
    ));
    assert_eq!(supervisor.state(), PrivateCodexSupervisorState::Running);
}

#[test]
fn shutdown_refuses_to_orphan_an_owned_process_or_create_terminal_facts() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = conversation(&ledger);
    save(&ledger, conversation_id, "prompt-1", "first");
    let mut supervisor = PrivateCodexSupervisor::new(host(&ledger, Runner::default()));
    assert!(matches!(
        supervisor.wake().unwrap(),
        PrivateCodexWake::Tick(tick)
            if matches!(tick.dispatch, Some(CodexPromptDispatchOutcome::Started { .. }))
    ));
    assert_eq!(
        supervisor.request_shutdown(),
        PrivateCodexShutdown::RefusedUndrained { active_runs: 1 }
    );
    assert_eq!(
        supervisor.state(),
        PrivateCodexSupervisorState::ShutdownRefused { active_runs: 1 }
    );
    assert_eq!(
        supervisor.wake().unwrap(),
        PrivateCodexWake::ShutdownRefused { active_runs: 1 }
    );
    assert!(ledger.find_event("codex:run-a:exit:1").unwrap().is_none());
}

#[test]
fn an_idle_supervisor_stops_without_claiming_or_recovering_new_work() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let mut supervisor = PrivateCodexSupervisor::new(host(&ledger, Runner::default()));
    assert_eq!(supervisor.request_shutdown(), PrivateCodexShutdown::Stopped);
    assert_eq!(supervisor.wake().unwrap(), PrivateCodexWake::Stopped);
}

fn host(
    ledger: &SqliteLedger,
    runner: Runner,
) -> ApprovedCodexHost<SqliteLedger, Runner, Resolver> {
    let compatibility = compatibility();
    let runtime = PublicDriversRuntime::new(
        profile(&compatibility),
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger.clone(),
        compatibility,
        runner,
        Resolver,
    )
    .unwrap();
    ApprovedCodexHost::new(runtime, "daemon-a".into(), None, HostEpoch(1), 1)
}

fn conversation(ledger: &SqliteLedger) -> AgentChatConversationId {
    let conversation_id = AgentChatConversationId("conversation-a".into());
    ledger
        .create_agent_chat_conversation(&AgentChatConversationCreate {
            receipt_id: ReceiptId("conversation-receipt".into()),
            idempotency_key: "conversation-key".into(),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            run_id: AgentChatRunId("run-a".into()),
            selection: selection(),
        })
        .unwrap();
    conversation_id
}

fn save(ledger: &SqliteLedger, conversation_id: AgentChatConversationId, id: &str, text: &str) {
    ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(id.into()),
            receipt_id: ReceiptId(format!("{id}-receipt")),
            host_epoch: HostEpoch(1),
            conversation_id,
            disposition: AgentChatPromptDisposition::Send,
            text: text.into(),
        })
        .unwrap();
}
