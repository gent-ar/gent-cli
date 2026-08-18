use gent_drivers::public_protocol::{PublicCompactionObservation, PublicWireFact};
use gent_types::{
    HostEpoch, NormalizedLifecycleSignal, NormalizedProviderEvent, RootActivity, TurnPhase,
};

use super::{NormalizedSessionFact, activity, batch, output, terminal, validate};

fn input(fact: PublicWireFact) -> NormalizedSessionFact {
    NormalizedSessionFact {
        run_id: "run-1".into(),
        conversation_id: "conversation-1".into(),
        turn_id: "turn-1".into(),
        host_epoch: HostEpoch(7),
        lifecycle_event_id: "lifecycle-1".into(),
        transcript_event_id: "transcript-1".into(),
        activity_event_id: "activity-1".into(),
        fact,
    }
}

#[test]
fn normalized_output_requires_a_transcript_record_but_not_activity() {
    let input = input(PublicWireFact::Event(NormalizedProviderEvent::Output {
        text: "normalized".into(),
        is_partial: true,
    }));
    assert_eq!(output(&input.fact), Some(("normalized".into(), true)));
    assert!(activity(&input).is_none());
    assert!(!terminal(&input.fact));
}

#[test]
fn terminal_lifecycle_produces_terminal_activity_and_never_settles_implicitly() {
    let input = input(PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::RootPhase {
            phase: TurnPhase::Ready,
        },
    ));
    assert!(terminal(&input.fact));
    assert!(matches!(
        activity(&input),
        Some(gent_types::ConversationActivityFact::Terminal { scope, phase: TurnPhase::Ready })
            if scope.cursor == 0 && scope.host_epoch == HostEpoch(7)
    ));
}

#[test]
fn daemon_constructs_an_atomic_batch_without_native_provider_fields() {
    let input = input(PublicWireFact::Event(NormalizedProviderEvent::Output {
        text: "normalized".into(),
        is_partial: true,
    }));
    let batch = batch("daemon-1", &input).unwrap();
    assert_eq!(batch.coordinator_id, "daemon-1");
    assert_eq!(
        batch.transcript.as_ref().map(|item| item.text.as_str()),
        Some("normalized")
    );
    assert!(batch.activity.is_none());
    assert_eq!(
        serde_json::to_value(batch).unwrap()["lifecycle"]["type"],
        "event"
    );
}

#[test]
fn compaction_stays_with_its_dedicated_ingress() {
    let compaction = input(PublicWireFact::Compaction(
        PublicCompactionObservation::Started,
    ));
    assert!(validate(&compaction).is_err());
    let nonterminal = input(PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::RootActivity {
            activity: RootActivity::Idle,
        },
    ));
    assert!(validate(&nonterminal).is_ok());
}
