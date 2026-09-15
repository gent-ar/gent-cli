use gent_core::Run;
use gent_ports::{Ledger, RunLifecycleFactLedger};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    CapabilitySet, ConversationRecord, DurableTurnPhase, Event, HostEpoch, NormalizedProviderEvent,
    NormalizedSessionLifecycle, ReceiptId, RunLifecycleFact, TurnRecord,
};

fn append(ledger: &SqliteLedger, event_id: &str, lifecycle: NormalizedSessionLifecycle) {
    let payload = match &lifecycle {
        NormalizedSessionLifecycle::Event { event } => {
            serde_json::json!({ "runId": "run-a", "event": event })
        }
        NormalizedSessionLifecycle::Signal { signal } => {
            serde_json::json!({ "runId": "run-a", "signal": signal })
        }
    };
    let source = ledger
        .append_event(&Event {
            cursor: 0,
            event_id: event_id.into(),
            receipt_id: ReceiptId(format!("receipt:{event_id}")),
            host_epoch: HostEpoch(1),
            kind: "providerLifecycle".into(),
            payload,
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
fn durable_failed_turn_overrides_stale_provider_liveness() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let coordinator = Coordinator::new(ledger.clone(), CapabilitySet::default());
    coordinator
        .create_conversation_run(
            &ConversationRecord {
                conversation_id: "conversation-a".into(),
            },
            &Run {
                id: "run-a".into(),
                parent_run_id: None,
                provider: "claurst".into(),
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
    append(
        &ledger,
        "started",
        NormalizedSessionLifecycle::Event {
            event: NormalizedProviderEvent::TurnStarted {
                turn_id: "turn-a".into(),
            },
        },
    );
    coordinator
        .transition_turn("turn-a", DurableTurnPhase::Active, DurableTurnPhase::Failed)
        .unwrap();

    let status = coordinator.conversation_status("conversation-a").unwrap();
    assert_eq!(status.runs[0].active_turn_id, None);
    let live = status.runs[0].live_status.as_ref().unwrap();
    assert!(!live.status.is_processing());
    assert!(!live.status.has_live_subagent_work());
    assert!(!live.status.has_live_command_work());
    assert!(!live.status.needs_attention());
    assert!(live.status.has_error());
}
