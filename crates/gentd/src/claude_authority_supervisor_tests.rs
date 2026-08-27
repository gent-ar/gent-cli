use crate::approved_claude_host::ApprovedClaudeHost;
use crate::claude_authority_supervisor::{
    PrivateClaudeEscalation, PrivateClaudeShutdown, PrivateClaudeSupervisor,
    PrivateClaudeSupervisorState, PrivateClaudeWake,
};
use crate::claude_prompt_lifecycle::ClaudePromptDispatchOutcome;
use crate::claude_prompt_lifecycle_tests::{Resolver, Runner, compatibility, profile, prompt};
use crate::public_driver_runtime::PublicDriversRuntime;
use gent_drivers::claude_runner::ClaudeRunnerEffect;
use gent_drivers::interrupt::ProcessTreeSignal;
use gent_ports::{AgentChatWorkspaceLedger, Ledger};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRunId, AgentChatSelection, CapabilitySet, HostEpoch, ReceiptId,
    WorkspaceRecord,
};

#[test]
fn first_wake_recovers_then_runs_one_bounded_tick() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let mut supervisor = PrivateClaudeSupervisor::new(host(&ledger, Runner::default()));
    assert_eq!(
        supervisor.state(),
        PrivateClaudeSupervisorState::AwaitingRecovery
    );
    assert!(matches!(
        supervisor.wake().unwrap(),
        PrivateClaudeWake::Tick(tick) if tick.dispatch == Some(ClaudePromptDispatchOutcome::Empty)
    ));
    assert_eq!(supervisor.state(), PrivateClaudeSupervisorState::Running);
}

#[test]
fn shutdown_signals_in_order_and_refuses_to_fake_terminal_settlement() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = conversation(&ledger);
    prompt(&ledger, &conversation_id, "a");
    let runner = Runner::default();
    let mut supervisor = PrivateClaudeSupervisor::new(host(&ledger, runner.clone()));
    let _ = supervisor.wake().unwrap();
    assert_eq!(
        supervisor.request_shutdown().unwrap(),
        PrivateClaudeShutdown::Draining {
            active_runs: 1,
            signal: ProcessTreeSignal::Interrupt,
        }
    );
    assert_eq!(
        supervisor.escalate_shutdown().unwrap(),
        PrivateClaudeEscalation::SignalSent(ProcessTreeSignal::Terminate)
    );
    assert_eq!(
        supervisor.escalate_shutdown().unwrap(),
        PrivateClaudeEscalation::SignalSent(ProcessTreeSignal::Kill)
    );
    assert_eq!(
        supervisor.escalate_shutdown().unwrap(),
        PrivateClaudeEscalation::RefusedUndrained { active_runs: 1 }
    );
    assert_eq!(
        runner.0.lock().unwrap().signals,
        vec![
            ProcessTreeSignal::Interrupt,
            ProcessTreeSignal::Terminate,
            ProcessTreeSignal::Kill,
        ]
    );
    assert!(
        ledger
            .find_event("claude:1:run-a:terminal:1")
            .unwrap()
            .is_none()
    );
    assert!(!supervisor.shutdown_complete());
}

#[test]
fn drain_only_stops_after_process_exit_is_persisted() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = conversation(&ledger);
    prompt(&ledger, &conversation_id, "a");
    let runner = Runner::default();
    let mut supervisor = PrivateClaudeSupervisor::new(host(&ledger, runner.clone()));
    let _ = supervisor.wake().unwrap();
    supervisor.request_shutdown().unwrap();
    runner
        .0
        .lock()
        .unwrap()
        .effects
        .push_back(vec![ClaudeRunnerEffect::Exited { code: Some(0) }]);
    assert!(matches!(
        supervisor.wake().unwrap(),
        PrivateClaudeWake::Drain(drain) if drain.exited_runs == 1
    ));
    assert_eq!(supervisor.state(), PrivateClaudeSupervisorState::Stopped);
    assert!(supervisor.shutdown_complete());
    assert!(
        ledger
            .find_event("claude:1:run-a:terminal:1")
            .unwrap()
            .is_some()
    );
}

#[test]
fn idle_shutdown_stops_without_recovery_or_claim() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let mut supervisor = PrivateClaudeSupervisor::new(host(&ledger, Runner::default()));
    assert_eq!(
        supervisor.request_shutdown().unwrap(),
        PrivateClaudeShutdown::Stopped
    );
    assert_eq!(supervisor.wake().unwrap(), PrivateClaudeWake::Stopped);
}

fn host(
    ledger: &SqliteLedger,
    runner: Runner,
) -> ApprovedClaudeHost<SqliteLedger, Runner, Resolver> {
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
    ApprovedClaudeHost::new(runtime, "daemon-a".into(), HostEpoch(1), 1, None)
}

fn conversation(ledger: &SqliteLedger) -> AgentChatConversationId {
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
                    provider: AgentChatProvider::Claude,
                    model: "claude-test".into(),
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
    conversation_id
}
