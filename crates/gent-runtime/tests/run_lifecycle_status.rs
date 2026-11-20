use gent_core::Run;
use gent_ports::{Ledger, RunLifecycleFactLedger};
use gent_runtime::{Coordinator, RunLifecycleStatusService};
use gent_store::SqliteLedger;
use gent_types::{
    CapabilitySet, ConversationRecord, DurableTurnPhase, Event, HostEpoch, NormalizedProviderEvent,
    NormalizedSessionLifecycle, ReceiptId, RunLifecycleFact, TurnRecord, WorkPhase,
};

fn service(ledger: SqliteLedger) -> RunLifecycleStatusService<SqliteLedger> {
    RunLifecycleStatusService::new(Coordinator::new(ledger, CapabilitySet::default()))
}

fn prepare(ledger: &SqliteLedger) -> Coordinator<SqliteLedger> {
    let coordinator = Coordinator::new(ledger.clone(), CapabilitySet::default());
    coordinator
        .create_conversation_run(
            &ConversationRecord {
                conversation_id: "conversation-a".into(),
            },
            &Run {
                id: "run-a".into(),
                parent_run_id: None,
                provider: "claude".into(),
            },
        )
        .unwrap();
    coordinator
        .create_turn(&TurnRecord {
            turn_id: "turn-a".into(),
            conversation_id: "conversation-a".into(),
            run_id: "run-a".into(),
            sequence: 1,
            phase: DurableTurnPhase::Active,
        })
        .unwrap();
    coordinator
}

fn append(ledger: &SqliteLedger, event_id: &str, event: NormalizedProviderEvent) {
    let source = ledger
        .append_event(&Event {
            cursor: 0,
            event_id: event_id.into(),
            receipt_id: ReceiptId(format!("receipt:{event_id}")),
            host_epoch: HostEpoch(1),
            kind: "providerLifecycle".into(),
            payload: serde_json::json!({ "runId": "run-a", "event": event }),
        })
        .unwrap();
    ledger
        .append_run_lifecycle_fact(&RunLifecycleFact {
            run_id: "run-a".into(),
            event_id: event_id.into(),
            host_epoch: HostEpoch(1),
            cursor: source.cursor,
            lifecycle: NormalizedSessionLifecycle::Event { event },
        })
        .unwrap();
}

fn turn_started(ledger: &SqliteLedger) {
    append(
        ledger,
        "started",
        NormalizedProviderEvent::TurnStarted {
            turn_id: "turn-a".into(),
        },
    );
}

fn child_started(ledger: &SqliteLedger) {
    append(
        ledger,
        "child",
        NormalizedProviderEvent::ChildStarted {
            child_id: "child-a".into(),
            parent_tool_use_id: "tool-a".into(),
        },
    );
}

fn complete(coordinator: &Coordinator<SqliteLedger>) {
    coordinator
        .transition_turn(
            "turn-a",
            DurableTurnPhase::Active,
            DurableTurnPhase::Completed,
        )
        .unwrap();
}

fn status(ledger: &SqliteLedger) -> gent_types::ConversationLiveStatus {
    service(ledger.clone())
        .live_status("run-a")
        .unwrap()
        .unwrap()
        .status
}

#[test]
fn status_reads_the_durable_turn_and_open_work_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    prepare(&ledger);
    turn_started(&ledger);
    child_started(&ledger);
    drop(ledger);

    let ledger = SqliteLedger::open(path).unwrap();
    let live = status(&ledger);
    assert!(live.is_processing());
    assert!(live.has_live_subagent_work());
    assert!(!live.is_waiting_for_subagents());
    assert_eq!(live.cursor, 2);
}

#[test]
fn a_waiting_turn_is_processing_and_needs_attention() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let coordinator = prepare(&ledger);
    turn_started(&ledger);
    coordinator
        .transition_turn(
            "turn-a",
            DurableTurnPhase::Active,
            DurableTurnPhase::WaitingQuestion,
        )
        .unwrap();
    let live = status(&ledger);
    assert!(live.is_processing());
    assert!(live.needs_attention());
}

#[test]
fn a_background_child_outlives_its_settled_root_turn_without_keeping_it_processing() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let coordinator = prepare(&ledger);
    turn_started(&ledger);
    child_started(&ledger);
    complete(&coordinator);
    let live = status(&ledger);
    assert!(!live.is_processing());
    assert!(live.is_waiting_for_subagents());
    assert!(!live.has_error());
    append(
        &ledger,
        "child-done",
        NormalizedProviderEvent::ChildTerminal {
            child_id: "child-a".into(),
            phase: WorkPhase::Done,
        },
    );
    assert!(!status(&ledger).has_live_subagent_work());
}

#[test]
fn a_provider_failure_marks_only_the_running_turn_as_failing() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let coordinator = prepare(&ledger);
    turn_started(&ledger);
    append(
        &ledger,
        "failure",
        NormalizedProviderEvent::ProviderFailure {
            classification: gent_types::ProviderFailureClassification::Provider,
            message: "boom".into(),
            resets_at: None,
        },
    );
    assert!(status(&ledger).has_error());
    complete(&coordinator);
    assert!(!status(&ledger).has_error());
}

fn run_status(coordinator: &Coordinator<SqliteLedger>) -> gent_types::ConversationRunStatus {
    coordinator
        .conversation_status("conversation-a")
        .unwrap()
        .runs
        .remove(0)
}

fn child_permission() -> gent_types::PermissionDecisionRequest {
    gent_types::PermissionDecisionRequest {
        binding: gent_types::PermissionDecisionBinding {
            decision_id: gent_types::AgentChatDecisionId("0".into()),
            request_idempotency_key: "claude:child-a:0".into(),
            conversation_id: gent_types::AgentChatConversationId("conversation-a".into()),
            run_id: gent_types::AgentChatRunId("run-a".into()),
            turn_id: "turn-a".into(),
            policy_id: "policy-1".into(),
            policy_revision: 1,
            host_epoch: HostEpoch(1),
            request_digest_sha256: gent_types::PermissionRequestDigest("a".repeat(64)),
        },
        request: gent_types::PermissionRequest {
            tool_name: "Bash".into(),
            category: gent_types::PermissionCategory::Command,
            input: None,
            child_id: Some("child-a".into()),
        },
    }
}

#[test]
fn a_background_child_permission_needs_attention_without_an_open_turn() {
    use gent_ports::PendingPermissionLedger;
    let ledger = SqliteLedger::in_memory().unwrap();
    let coordinator = prepare(&ledger);
    turn_started(&ledger);
    child_started(&ledger);
    complete(&coordinator);
    ledger.save_pending_permission(&child_permission()).unwrap();
    let run = run_status(&coordinator);
    assert_eq!(run.active_turn_id, None);
    let live = run.live_status.unwrap().status;
    assert!(!live.is_processing());
    assert!(live.needs_attention());
    assert!(live.is_waiting_for_subagents());
}

#[test]
fn an_open_provider_turn_is_processing_and_a_settled_one_is_not() {
    use gent_ports::NormalizedSessionBatchLedger;
    let ledger = SqliteLedger::in_memory().unwrap();
    let coordinator = prepare(&ledger);
    turn_started(&ledger);
    child_started(&ledger);
    complete(&coordinator);
    let provider_turn = ledger
        .start_provider_turn("conversation-a", "run-a", HostEpoch(1))
        .unwrap();
    let run = run_status(&coordinator);
    assert_eq!(run.active_turn_id.as_deref(), Some(provider_turn.as_str()));
    assert!(run.live_status.unwrap().status.is_processing());
    coordinator
        .transition_turn(
            &provider_turn,
            DurableTurnPhase::Active,
            DurableTurnPhase::Completed,
        )
        .unwrap();
    let run = run_status(&coordinator);
    assert_eq!(run.active_turn_id, None);
    let live = run.live_status.unwrap().status;
    assert!(!live.is_processing());
    assert!(!live.needs_attention());
    assert!(live.is_waiting_for_subagents());
}
