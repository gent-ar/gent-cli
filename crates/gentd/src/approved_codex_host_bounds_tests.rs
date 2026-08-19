use crate::approved_codex_host::ApprovedCodexHost;
use crate::codex_prompt_lifecycle::CodexPromptDispatchOutcome;
use crate::codex_prompt_lifecycle_tests::{Resolver, Runner, compatibility, profile, selection};
use crate::public_driver_runtime::PublicDriversRuntime;
use gent_drivers::codex_runner::CodexRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{AgentChatPromptLedger, AgentChatWorkspaceLedger, Ledger};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatPromptCreate,
    AgentChatPromptDisposition, AgentChatRequestId, AgentChatRunId, CapabilitySet, HostEpoch,
    NormalizedLifecycleSignal, ReceiptId, TurnPhase, WorkspaceRecord,
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
fn approved_host_preserves_owned_process_after_an_unproven_poll_failure() {
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

    let error = host.tick().unwrap_err();
    assert!(error.to_string().contains("provider poll unavailable"));
    assert!(ledger.find_event("codex:run-a:poll:1").unwrap().is_none());
    host.signal_active(gent_drivers::interrupt::ProcessTreeSignal::Interrupt)
        .unwrap();
    assert_eq!(
        runner.state.lock().unwrap().signals,
        [gent_drivers::interrupt::ProcessTreeSignal::Interrupt]
    );
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
    ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(id.into()),
            receipt_id: ReceiptId(format!("{id}-receipt")),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            disposition: AgentChatPromptDisposition::Send,
            text: text.into(),
        })
        .unwrap();
}
