use gent_ports::{AgentChatPromptLedger, AgentChatWorkspaceLedger, Ledger};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, CapabilitySet, HostEpoch, NormalizedLifecycleSignal,
    ReceiptId, TurnPhase, WorkspaceRecord,
};

use gent_drivers::codex_runner::CodexRunnerEffect;
use gent_drivers::public_protocol::{PublicCompactionObservation, PublicWireFact};

use crate::codex_prompt_lifecycle::{CodexPromptDispatchOutcome, CodexPromptLifecycle};
use crate::codex_prompt_lifecycle_tests::{Resolver, Runner, compatibility, profile};
use crate::public_driver_runtime::PublicDriversRuntime;

type Host = CodexPromptLifecycle<SqliteLedger, Runner, Resolver>;

fn started_host(directory: &std::path::Path) -> (SqliteLedger, Runner, Host) {
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
            attachment_ids: vec![],
            tool_source_ids: vec![],
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
    .unwrap()
    .with_attachment_roots(
        directory.join("attachments"),
        directory.join("codex-attachments"),
    );
    let mut host = CodexPromptLifecycle::new(runtime, "daemon-a".into());
    assert!(matches!(
        host.dispatch_next(HostEpoch(1)).unwrap(),
        CodexPromptDispatchOutcome::Started { .. }
    ));
    (ledger, runner, host)
}

#[test]
fn codex_poll_failure_retains_ownership_without_fabricating_terminal_settlement() {
    let directory = tempfile::tempdir().unwrap();
    let (ledger, runner, mut host) = started_host(directory.path());
    host.interrupt("run-a").unwrap();
    assert_eq!(runner.state.lock().unwrap().turn_interrupts, ["run-a"]);
    assert!(runner.state.lock().unwrap().signals.is_empty());
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

#[test]
fn codex_compaction_observations_never_fail_the_owned_turn() {
    let directory = tempfile::tempdir().unwrap();
    let (_ledger, runner, mut host) = started_host(directory.path());
    runner.state.lock().unwrap().effects.push_back(vec![
        CodexRunnerEffect::Fact(PublicWireFact::SessionStarted {
            provider_session_id: "thread-a".into(),
        }),
        CodexRunnerEffect::Fact(PublicWireFact::Compaction(
            PublicCompactionObservation::Started,
        )),
        CodexRunnerEffect::Fact(PublicWireFact::Compaction(
            PublicCompactionObservation::Completed,
        )),
    ]);
    let batch = host.poll_active(HostEpoch(1), 16).unwrap();
    assert_eq!(batch.polled_runs, 1);
    assert_eq!(batch.exited_runs, 0, "compaction must not fail the run");
    assert_eq!(batch.facts, 3);
    runner
        .state
        .lock()
        .unwrap()
        .effects
        .push_back(vec![CodexRunnerEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::RootPhase {
                phase: TurnPhase::Ready,
            },
        ))]);
    let settled = host.poll_active(HostEpoch(1), 16).unwrap();
    assert_eq!((settled.facts, settled.exited_runs), (1, 0));
}
