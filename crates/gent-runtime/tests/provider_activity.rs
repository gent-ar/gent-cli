use gent_ports::{
    ConversationActivityLedger, ConversationLedger, Ledger, RunLease, RunRecord, RunSessionBinding,
};
use gent_runtime::{
    ConversationActivityAuthority, ConversationActivityResult, ConversationActivityService,
    Coordinator, ProviderActivityFact, ProviderActivityIngress, ProviderRunAuthority,
};
use gent_store::SqliteLedger;
use gent_types::{
    CapabilitySet, ConversationActivityFact, ConversationActivityScope, ConversationRecord,
    HostEpoch,
};

fn fact(event_id: &str) -> ProviderActivityFact {
    ProviderActivityFact {
        event_id: event_id.into(),
        activity: ConversationActivityFact::TurnStarted {
            scope: ConversationActivityScope {
                conversation_id: "conversation-a".into(),
                run_id: "run-a".into(),
                turn_id: "turn-a".into(),
                host_epoch: HostEpoch(1),
                cursor: 0,
            },
        },
    }
}

fn prepare(ledger: &SqliteLedger) {
    ledger
        .create_conversation_run(
            &ConversationRecord {
                conversation_id: "conversation-a".into(),
            },
            &RunRecord {
                run_id: "run-a".into(),
                parent_run_id: None,
                provider: "claude".into(),
            },
        )
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
            provider_session_id: "native-a".into(),
        })
        .unwrap();
}

fn ingress(
    ledger: SqliteLedger,
    activity_authority: ConversationActivityAuthority,
    authority: ProviderRunAuthority,
) -> ProviderActivityIngress<SqliteLedger> {
    ProviderActivityIngress::new(
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ConversationActivityService::new(ledger, activity_authority),
        authority,
    )
}

#[test]
fn approved_owned_fact_uses_its_durable_source_cursor_and_retries_safely() {
    let ledger = SqliteLedger::in_memory().unwrap();
    prepare(&ledger);
    let service = ingress(
        ledger.clone(),
        ConversationActivityAuthority::Approved,
        ProviderRunAuthority::PublicDrivers,
    );

    let ConversationActivityResult::Recorded(activity) =
        service.record("daemon-a", fact("a-1")).unwrap()
    else {
        panic!("owned fact must persist")
    };
    assert_eq!(activity.scope().cursor, 1);
    assert_eq!(activity.scope().conversation_id, "conversation-a");
    assert_eq!(activity.scope().run_id, "run-a");
    assert_eq!(activity.scope().host_epoch, HostEpoch(1));
    assert!(matches!(
        service.record("daemon-a", fact("a-1")).unwrap(),
        ConversationActivityResult::Recorded(retry) if retry.scope().cursor == 1
    ));

    let events = ledger.read_event_page(0, 100).unwrap().events;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "providerActivity");
    assert_eq!(events[0].payload["conversationId"], "conversation-a");
    assert_eq!(events[0].payload["activity"]["type"], "turnStarted");
    assert!(
        ledger
            .read_conversation_activity_page("conversation-a", "run-a", 0, 64)
            .unwrap()
            .facts
            .len()
            == 1
    );
}

#[test]
fn observer_rejection_does_not_append_a_source_or_fact() {
    let ledger = SqliteLedger::in_memory().unwrap();
    prepare(&ledger);
    let service = ingress(
        ledger.clone(),
        ConversationActivityAuthority::Approved,
        ProviderRunAuthority::Observer,
    );

    assert!(service.record("daemon-a", fact("denied")).is_err());
    let events = ledger.read_event_page(0, 100).unwrap().events;
    assert!(events.is_empty());
    assert!(
        ledger
            .read_conversation_activity_page("conversation-a", "run-a", 0, 64)
            .unwrap()
            .facts
            .is_empty()
    );
}

#[test]
fn fact_needs_owned_session_and_a_source_allocated_cursor() {
    let ledger = SqliteLedger::in_memory().unwrap();
    prepare(&ledger);
    let service = ingress(
        ledger.clone(),
        ConversationActivityAuthority::Approved,
        ProviderRunAuthority::PublicDrivers,
    );
    assert!(service.record("different-daemon", fact("unowned")).is_err());

    let mut caller_cursor = fact("caller-cursor");
    if let ConversationActivityFact::TurnStarted { scope } = &mut caller_cursor.activity {
        scope.cursor = 9;
    }
    assert!(service.record("daemon-a", caller_cursor).is_err());
    let events = ledger.read_event_page(0, 100).unwrap().events;
    assert!(events.is_empty());
}
