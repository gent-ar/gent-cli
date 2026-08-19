use gent_ports::{Ledger, RunLifecycleFactLedger, RunRecord};
use gent_runtime::{Coordinator, RunLifecycleStatusService};
use gent_store::SqliteLedger;
use gent_types::{
    CapabilitySet, Event, HostEpoch, NormalizedLifecycleSignal, NormalizedProviderEvent,
    NormalizedSessionLifecycle, ReceiptId, RunLifecycleFact, TurnPhase,
};

fn service(ledger: SqliteLedger) -> RunLifecycleStatusService<SqliteLedger> {
    RunLifecycleStatusService::new(Coordinator::new(ledger, CapabilitySet::default()))
}

fn prepare(ledger: &SqliteLedger) {
    ledger
        .create_run(&RunRecord {
            run_id: "run-a".into(),
            parent_run_id: None,
            provider: "claude".into(),
        })
        .unwrap();
}

fn append(ledger: &SqliteLedger, event_id: &str, lifecycle: NormalizedSessionLifecycle) {
    let source = ledger
        .append_event(&Event {
            cursor: 0,
            event_id: event_id.into(),
            receipt_id: ReceiptId(format!("receipt:{event_id}")),
            host_epoch: HostEpoch(1),
            kind: "normalizedSessionLifecycle".into(),
            payload: serde_json::json!({ "runId": "run-a", "lifecycle": lifecycle }),
        })
        .unwrap();
    ledger
        .append_run_lifecycle_fact(&RunLifecycleFact {
            run_id: "run-a".into(),
            event_id: event_id.into(),
            host_epoch: HostEpoch(1),
            cursor: source.cursor,
            lifecycle,
        })
        .unwrap();
}

#[test]
fn status_replays_lifecycle_facts_after_restart_without_a_projection() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    prepare(&ledger);
    append(
        &ledger,
        "started",
        NormalizedSessionLifecycle::Event {
            event: NormalizedProviderEvent::TurnStarted {
                turn_id: "turn-a".into(),
            },
        },
    );
    append(
        &ledger,
        "child",
        NormalizedSessionLifecycle::Event {
            event: NormalizedProviderEvent::ChildStarted {
                child_id: "child-a".into(),
                parent_tool_use_id: "tool-a".into(),
            },
        },
    );
    drop(ledger);

    let status = service(SqliteLedger::open(path).unwrap())
        .live_status("run-a")
        .unwrap()
        .unwrap();
    assert!(status.status.is_processing);
    assert!(status.status.has_live_subagent_work);
    assert_eq!(status.status.cursor, 2);
}

#[test]
fn status_reduces_signals_in_cursor_order() {
    let ledger = SqliteLedger::in_memory().unwrap();
    prepare(&ledger);
    append(
        &ledger,
        "waiting",
        NormalizedSessionLifecycle::Signal {
            signal: NormalizedLifecycleSignal::RootPhase {
                phase: TurnPhase::WaitingQuestion,
            },
        },
    );
    append(
        &ledger,
        "attention",
        NormalizedSessionLifecycle::Signal {
            signal: NormalizedLifecycleSignal::AttentionRequired,
        },
    );
    let status = service(ledger).live_status("run-a").unwrap().unwrap();
    assert!(status.status.is_processing);
    assert!(status.status.needs_attention);
}
