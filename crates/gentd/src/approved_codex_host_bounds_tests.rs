use crate::approved_codex_host::ApprovedCodexHost;
use crate::codex_prompt_lifecycle::CodexPromptDispatchOutcome;
use crate::codex_prompt_lifecycle_tests::{Resolver, Runner, compatibility, profile, selection};
use crate::public_driver_runtime::PublicDriversRuntime;
use gent_drivers::codex_control::CodexControlRequest;
use gent_drivers::codex_runner::CodexRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{
    AgentChatPromptLedger, AgentChatWorkspaceLedger, ConversationActivityLedger,
    ConversationLedger, PendingPermissionLedger,
};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatPromptCreate,
    AgentChatPromptDisposition, AgentChatRequestId, AgentChatRunId, CapabilitySet,
    ConversationActivityFact, DurableTurnPhase, HostEpoch, NormalizedLifecycleSignal, ReceiptId,
    TurnPhase, WorkspaceRecord,
};

#[test]
fn approved_host_polls_before_claiming_one_queued_follow_up_at_a_time() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = setup_conversation(&ledger);
    save(&ledger, &conversation_id, "prompt-1", "first");
    let runner = Runner::default();
    let mut host = host(&ledger, runner.clone());

    assert!(matches!(
        host.tick().unwrap().dispatch,
        Some(CodexPromptDispatchOutcome::Started { .. })
    ));
    runner.state.lock().unwrap().effects.push_back(vec![
        CodexRunnerEffect::Fact(PublicWireFact::SessionStarted {
            provider_session_id: "daemon-owned-session".into(),
        }),
        CodexRunnerEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::RootPhase {
                phase: TurnPhase::Ready,
            },
        )),
    ]);

    let tick = host.tick().unwrap();
    assert_eq!(tick.polled_runs, 1);
    assert_eq!(tick.facts, 2);
    host.signal_active(gent_drivers::interrupt::ProcessTreeSignal::Interrupt)
        .unwrap();
    assert_eq!(
        runner.state.lock().unwrap().signals,
        [gent_drivers::interrupt::ProcessTreeSignal::Interrupt],
        "a terminal turn fact must not discard the owned Codex process"
    );
    save(&ledger, &conversation_id, "prompt-2", "second");
    save(&ledger, &conversation_id, "prompt-3", "third");

    let tick = host.tick().unwrap();
    assert!(matches!(
        tick.dispatch,
        Some(CodexPromptDispatchOutcome::Started { .. })
    ));
    let state = runner.state.lock().unwrap();
    assert_eq!(state.starts, 1, "a follow-up must reuse the owned session");
    assert_eq!(state.submitted, ["second"]);
    drop(state);

    let tick = host.tick().unwrap();
    assert_eq!(tick.polled_runs, 1);
    assert_eq!(
        tick.dispatch, None,
        "an active follow-up blocks another claim"
    );
    assert_eq!(runner.state.lock().unwrap().submitted, ["second"]);
}

#[test]
fn approved_host_fails_only_the_turn_whose_process_poll_failed() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = setup_conversation(&ledger);
    save(&ledger, &conversation_id, "prompt-1", "first");
    let runner = Runner::default();
    let mut host = host(&ledger, runner.clone());
    assert!(matches!(
        host.tick().unwrap().dispatch,
        Some(CodexPromptDispatchOutcome::Started { .. })
    ));
    runner.state.lock().unwrap().poll_failure = true;

    let tick = host.tick().unwrap();
    assert_eq!(tick.exited_runs, 1);
    assert_eq!(
        ledger.list_run_turns("run-a").unwrap()[0].phase,
        DurableTurnPhase::Failed
    );
    assert!(
        ledger
            .read_conversation_activity_page(&conversation_id.0, "run-a", 0, 64)
            .unwrap()
            .facts
            .iter()
            .any(|fact| matches!(
                fact,
                ConversationActivityFact::Terminal {
                    phase: TurnPhase::Failed,
                    ..
                }
            ))
    );
    assert_eq!(runner.state.lock().unwrap().releases, ["run-a"]);
    assert_eq!(
        host.signal_active(gent_drivers::interrupt::ProcessTreeSignal::Interrupt)
            .unwrap(),
        0
    );
}

#[test]
fn interrupt_with_a_pending_approval_settles_and_a_restarted_process_can_ask_again() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = setup_conversation(&ledger);
    let run_id = AgentChatRunId("run-a".into());
    save(&ledger, &conversation_id, "prompt-1", "first");
    let runner = Runner::default();
    let mut host = host(&ledger, runner.clone());
    let push = |effects: Vec<CodexRunnerEffect>| {
        runner.state.lock().unwrap().effects.push_back(effects);
    };
    let approval = || {
        CodexRunnerEffect::ControlRequest(CodexControlRequest {
            request_id: serde_json::json!(0),
            request_key: "0".into(),
            method: "item/commandExecution/requestApproval".into(),
            tool_use_id: "call-1".into(),
            tool_name: "Bash".into(),
            input: None,
        })
    };
    let root_phase = |phase| {
        CodexRunnerEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::RootPhase { phase },
        ))
    };
    host.tick().unwrap();
    push(vec![
        CodexRunnerEffect::Fact(PublicWireFact::SessionStarted {
            provider_session_id: "thread-1".into(),
        }),
        approval(),
    ]);
    host.tick().unwrap();
    let first = ledger
        .pending_permission(&conversation_id, &run_id)
        .unwrap()
        .unwrap();

    host.interrupt("run-a").unwrap();
    push(vec![root_phase(TurnPhase::Interrupted)]);
    host.tick().unwrap();
    push(vec![CodexRunnerEffect::Exited { code: Some(0) }]);
    host.tick().unwrap();

    let turns = ledger.list_run_turns("run-a").unwrap();
    assert_eq!(turns[0].phase, DurableTurnPhase::Interrupted);
    assert!(
        ledger
            .pending_permission(&conversation_id, &run_id)
            .unwrap()
            .is_none()
    );
    save(&ledger, &conversation_id, "prompt-2", "second");
    host.tick().unwrap();
    push(vec![approval()]);
    host.tick().unwrap();
    let second = ledger
        .pending_permission(&conversation_id, &run_id)
        .unwrap()
        .unwrap();
    assert_eq!(second.binding.decision_id, first.binding.decision_id);
    assert_ne!(second.binding.turn_id, first.binding.turn_id);
    assert_ne!(
        second.binding.request_idempotency_key,
        first.binding.request_idempotency_key
    );
    push(vec![root_phase(TurnPhase::Ready)]);
    host.tick().unwrap();
    assert_eq!(
        ledger.list_run_turns("run-a").unwrap()[1].phase,
        DurableTurnPhase::Completed
    );
    let state = runner.state.lock().unwrap();
    assert_eq!(state.turn_interrupts, ["run-a"]);
    assert_eq!((state.starts, state.resumes), (1, 1));
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
    ApprovedCodexHost::new(runtime, "daemon-a".into(), HostEpoch(1), 1)
}

fn setup_conversation(ledger: &SqliteLedger) -> AgentChatConversationId {
    let conversation_id = AgentChatConversationId("conversation-a".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation_id.clone(),
                run_id: AgentChatRunId("run-a".into()),
                selection: selection(),
            },
            &WorkspaceRecord {
                workspace_id: "workspace-a".into(),
                canonical_path: "/workspace-a".into(),
            },
        )
        .unwrap();
    conversation_id
}

fn save(ledger: &SqliteLedger, conversation_id: &AgentChatConversationId, id: &str, text: &str) {
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(id.into()),
            receipt_id: ReceiptId(format!("{id}-receipt")),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            disposition: AgentChatPromptDisposition::Send,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: text.into(),
        })
        .unwrap();
    crate::readiness_test_support::release(ledger, &saved);
}
