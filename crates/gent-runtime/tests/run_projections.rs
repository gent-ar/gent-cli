use gent_ports::{Ledger, RunLease, RunProjectionLedger, RunRecord, RunSessionBinding};
use gent_runtime::{Coordinator, RunProjectionService, RuntimeError};
use gent_store::SqliteLedger;
use gent_types::{CapabilitySet, HostEpoch, NormalizedProviderEvent};

fn service(ledger: SqliteLedger) -> RunProjectionService<SqliteLedger> {
    RunProjectionService::new(Coordinator::new(ledger, CapabilitySet::default()))
}

fn prepare(ledger: &SqliteLedger) {
    ledger
        .create_run(&RunRecord {
            run_id: "run-a".into(),
            parent_run_id: None,
            provider: "claude".into(),
        })
        .unwrap();
    ledger
        .claim_run_lease(&RunLease {
            run_id: "run-a".into(),
            coordinator_id: "daemon-a".into(),
            host_epoch: HostEpoch(1),
        })
        .unwrap();
    ledger
        .save_run_session_binding(&RunSessionBinding {
            run_id: "run-a".into(),
            provider_session_id: "private-to-daemon".into(),
        })
        .unwrap();
}

#[test]
fn owned_session_bound_events_persist_a_replay_safe_live_status() {
    let ledger = SqliteLedger::in_memory().unwrap();
    prepare(&ledger);
    let service = service(ledger.clone());
    service
        .record_normalized_event(
            "run-a".into(),
            "daemon-a",
            HostEpoch(1),
            1,
            &NormalizedProviderEvent::TurnStarted {
                turn_id: "turn-a".into(),
            },
        )
        .unwrap();
    service
        .record_normalized_event(
            "run-a".into(),
            "daemon-a",
            HostEpoch(1),
            2,
            &NormalizedProviderEvent::ChildStarted {
                child_id: "child-a".into(),
                parent_tool_use_id: "tool-a".into(),
            },
        )
        .unwrap();
    let status = service
        .record_normalized_event(
            "run-a".into(),
            "daemon-a",
            HostEpoch(1),
            3,
            &NormalizedProviderEvent::TurnEnded {
                turn_id: "turn-a".into(),
            },
        )
        .unwrap();
    assert!(status.status.is_waiting_for_subagents);
    assert_eq!(status.status.snapshot_cursor, 3);

    let stale = service
        .record_normalized_event(
            "run-a".into(),
            "daemon-a",
            HostEpoch(1),
            2,
            &NormalizedProviderEvent::ChildTerminal {
                child_id: "child-a".into(),
                phase: gent_types::WorkPhase::Done,
            },
        )
        .unwrap();
    assert!(stale.status.has_live_subagent_work);
    assert_eq!(service.live_status("run-a").unwrap(), Some(status));
    assert!(ledger.find_run_projection("run-a").unwrap().is_some());
}

#[test]
fn projection_requires_current_owner_and_server_owned_session() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_run(&RunRecord {
            run_id: "run-a".into(),
            parent_run_id: None,
            provider: "claude".into(),
        })
        .unwrap();
    let service = service(ledger);
    let error = service.record_normalized_event(
        "run-a".into(),
        "wrong-daemon",
        HostEpoch(1),
        1,
        &NormalizedProviderEvent::Output {
            text: "ignored".into(),
        },
    );
    assert!(matches!(error, Err(RuntimeError::Ledger(_))));
}

#[test]
fn projection_refuses_a_leased_run_without_a_server_owned_session() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_run(&RunRecord {
            run_id: "run-a".into(),
            parent_run_id: None,
            provider: "claude".into(),
        })
        .unwrap();
    ledger
        .claim_run_lease(&RunLease {
            run_id: "run-a".into(),
            coordinator_id: "daemon-a".into(),
            host_epoch: HostEpoch(1),
        })
        .unwrap();
    let error = service(ledger).record_normalized_event(
        "run-a".into(),
        "daemon-a",
        HostEpoch(1),
        1,
        &NormalizedProviderEvent::Output {
            text: "ignored".into(),
        },
    );
    assert!(matches!(error, Err(RuntimeError::Ledger(_))));
}
