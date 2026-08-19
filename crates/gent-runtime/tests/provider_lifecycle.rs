use gent_ports::{Ledger, RunLease, RunLifecycleFactLedger, RunRecord};
use gent_runtime::{
    Coordinator, ProviderLifecycleEffect, ProviderLifecycleIngress, ProviderRunAuthority,
};
use gent_store::SqliteLedger;
use gent_types::{
    CapabilitySet, DecisionCommand, DecisionSettlementPhase, HostEpoch, NormalizedProviderEvent,
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
fn source_event_precedes_session_and_immutable_lifecycle_fact_updates() {
    let ledger = SqliteLedger::in_memory().unwrap();
    prepare(&ledger);
    let service = ingress(ledger.clone(), ProviderRunAuthority::PublicDrivers);
    service
        .record(
            "source-session".into(),
            "run-a",
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
                "run-a",
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
            "run-a",
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
            "run-a",
            "daemon-a",
            HostEpoch(1),
            ProviderLifecycleEffect::Normalized(NormalizedProviderEvent::TurnStarted {
                turn_id: "turn-a".into(),
            }),
        )
        .unwrap()
        .unwrap();
    assert_eq!(status.status.cursor, 2);
    assert_eq!(
        ledger
            .find_run_session_binding("run-a")
            .unwrap()
            .unwrap()
            .provider_session_id,
        "native-a"
    );
    assert_eq!(
        ledger
            .read_run_lifecycle_fact_page("run-a", 0, 64)
            .unwrap()
            .facts
            .len(),
        1
    );
    let events = ledger.read_event_page(0, 100).unwrap().events;
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
                "run-a",
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
            "run-a",
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
            "run-a",
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
