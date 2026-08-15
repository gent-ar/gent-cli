use gent_drivers::SessionEffect;
use gent_ports::{Ledger, RunLease, RunProjectionLedger, RunRecord};
use gent_runtime::{
    Coordinator, ProviderEffectDispatcher, ProviderLifecycleEffect, ProviderLifecycleIngress,
    ProviderRunAuthority,
};
use gent_store::SqliteLedger;
use gent_types::{
    CapabilitySet, DecisionCommand, DecisionSettlementPhase, EventResume, HostEpoch,
    NormalizedProviderEvent,
};

fn ingress(
    ledger: SqliteLedger,
    authority: ProviderRunAuthority,
) -> ProviderLifecycleIngress<SqliteLedger> {
    ProviderLifecycleIngress::new(
        Coordinator::new(ledger, CapabilitySet::default()),
        authority,
    )
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
}

#[test]
fn source_event_precedes_session_and_projection_updates() {
    let ledger = SqliteLedger::in_memory().unwrap();
    prepare(&ledger);
    let service = ingress(ledger.clone(), ProviderRunAuthority::PublicDrivers);
    service
        .record(
            "source-session".into(),
            "run-a".into(),
            "daemon-a",
            HostEpoch(1),
            ProviderLifecycleEffect::SessionStarted {
                provider_session_id: "native-a".into(),
            },
        )
        .unwrap();
    assert!(
        service
            .record(
                "source-session".into(),
                "run-a".into(),
                "daemon-a",
                HostEpoch(1),
                ProviderLifecycleEffect::SessionStarted {
                    provider_session_id: "substituted".into(),
                },
            )
            .is_err()
    );
    service
        .record(
            "source-session".into(),
            "run-a".into(),
            "daemon-a",
            HostEpoch(1),
            ProviderLifecycleEffect::SessionStarted {
                provider_session_id: "native-a".into(),
            },
        )
        .unwrap();
    let status = service
        .record(
            "source-turn".into(),
            "run-a".into(),
            "daemon-a",
            HostEpoch(1),
            ProviderLifecycleEffect::Normalized(NormalizedProviderEvent::TurnStarted {
                turn_id: "turn-a".into(),
            }),
        )
        .unwrap()
        .unwrap();
    assert_eq!(status.status.snapshot_cursor, 2);
    assert_eq!(
        ledger
            .find_run_session_binding("run-a")
            .unwrap()
            .unwrap()
            .provider_session_id,
        "native-a"
    );
    assert!(ledger.find_run_projection("run-a").unwrap().is_some());
    let EventResume::Delta { events } = ledger.resume_events(0).unwrap() else {
        panic!("a fresh provider source read cannot require resync");
    };
    assert_eq!(
        events.len(),
        2,
        "a retried source event is not appended twice"
    );
    assert_eq!(events[0].payload["effect"], "sessionStarted");
    assert!(!events[0].payload.to_string().contains("native-a"));
    assert_eq!(events[1].payload["event"]["type"], "turnStarted");
}

#[test]
fn provider_settlement_is_daemon_owned_and_observer_is_hard_disabled() {
    let ledger = SqliteLedger::in_memory().unwrap();
    prepare(&ledger);
    let coordinator = Coordinator::new(ledger.clone(), CapabilitySet::default());
    coordinator
        .submit_decision(DecisionCommand {
            decision_id: "decision-a".into(),
            idempotency_key: "key-a".into(),
        })
        .unwrap();
    let observer = ingress(ledger.clone(), ProviderRunAuthority::Observer);
    assert!(
        observer
            .record(
                "denied".into(),
                "run-a".into(),
                "daemon-a",
                HostEpoch(1),
                ProviderLifecycleEffect::SessionStarted {
                    provider_session_id: "native-a".into(),
                },
            )
            .is_err()
    );
    let service = ingress(ledger.clone(), ProviderRunAuthority::PublicDrivers);
    service
        .record(
            "session".into(),
            "run-a".into(),
            "daemon-a",
            HostEpoch(1),
            ProviderLifecycleEffect::SessionStarted {
                provider_session_id: "native-a".into(),
            },
        )
        .unwrap();
    service
        .record(
            "settled".into(),
            "run-a".into(),
            "daemon-a",
            HostEpoch(1),
            ProviderLifecycleEffect::ProviderSettled {
                decision_id: "decision-a".into(),
            },
        )
        .unwrap();
    assert_eq!(
        ledger.find_decision("decision-a").unwrap().unwrap().phase,
        DecisionSettlementPhase::Settled
    );
}

#[test]
fn dispatcher_persists_reduced_driver_facts_but_ignores_process_local_retries() {
    let ledger = SqliteLedger::in_memory().unwrap();
    prepare(&ledger);
    let dispatcher = ProviderEffectDispatcher::new(
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ProviderRunAuthority::PublicDrivers,
    );
    assert_eq!(
        dispatcher
            .record(
                "ignored-retry".into(),
                "run-a".into(),
                "daemon-a",
                HostEpoch(1),
                &SessionEffect::StartAttempt { attempt: 2 },
            )
            .unwrap(),
        None
    );
    dispatcher
        .record(
            "driver-session".into(),
            "run-a".into(),
            "daemon-a",
            HostEpoch(1),
            &SessionEffect::SessionStarted {
                provider_session_id: "native-session".into(),
            },
        )
        .unwrap();
    let status = dispatcher
        .record(
            "driver-turn".into(),
            "run-a".into(),
            "daemon-a",
            HostEpoch(1),
            &SessionEffect::Normalized {
                event: NormalizedProviderEvent::TurnStarted {
                    turn_id: "turn-a".into(),
                },
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(status.status.snapshot_cursor, 2);
    assert_eq!(
        dispatcher
            .record(
                "driver-terminal".into(),
                "run-a".into(),
                "daemon-a",
                HostEpoch(1),
                &SessionEffect::Terminal {
                    reason: "completed".into(),
                },
            )
            .unwrap(),
        None
    );
    assert_eq!(
        ledger
            .find_run_session_binding("run-a")
            .unwrap()
            .unwrap()
            .provider_session_id,
        "native-session"
    );
    let EventResume::Delta { events } = ledger.resume_events(0).unwrap() else {
        panic!("driver facts must remain cursor-resumable");
    };
    assert_eq!(events.len(), 3);
    assert!(!events[0].payload.to_string().contains("native-session"));
}
