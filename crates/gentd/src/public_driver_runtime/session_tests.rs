use gent_drivers::public_protocol::{PublicCompactionObservation, PublicWireFact};
use gent_types::{
    HostEpoch, NormalizedLifecycleSignal, NormalizedProviderEvent, ProviderFailureClassification,
    RootActivity, ToolActivity, ToolPhase, TurnPhase, WorkPhase,
};

use super::{NormalizedSessionFact, activity, batch, terminal, transcript_content, validate};

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
    assert_eq!(
        transcript_content(&input.fact),
        Some((
            gent_types::NormalizedTranscriptKind::AssistantMessage,
            "normalized".into(),
            true
        ))
    );
    assert!(activity(&input).is_none());
    assert!(!terminal(&input.fact));
}

#[test]
fn normalized_thinking_is_a_durable_disclosure_transcript_not_assistant_output() {
    let input = input(PublicWireFact::Event(NormalizedProviderEvent::Thinking {
        text: "considering the plan".into(),
        is_partial: true,
    }));
    assert_eq!(
        transcript_content(&input.fact),
        Some((
            gent_types::NormalizedTranscriptKind::Thinking,
            "considering the plan".into(),
            true
        ))
    );
    let batch = batch("daemon-1", &input).unwrap();
    assert_eq!(
        batch.transcript.as_ref().map(|item| item.kind),
        Some(gent_types::NormalizedTranscriptKind::Thinking)
    );
    assert!(batch.activity.is_none());
}

#[test]
fn normalized_tool_output_is_a_durable_tool_activity_transcript() {
    let input = input(PublicWireFact::Event(
        NormalizedProviderEvent::ToolOutputDelta {
            tool_use_id: "tool-1".into(),
            text: "stdout chunk".into(),
            is_partial: true,
        },
    ));
    assert_eq!(
        transcript_content(&input.fact),
        Some((
            gent_types::NormalizedTranscriptKind::ToolActivity,
            "stdout chunk".into(),
            true,
        ))
    );
    assert_eq!(
        batch("daemon-1", &input)
            .unwrap()
            .transcript
            .as_ref()
            .map(|item| item.kind),
        Some(gent_types::NormalizedTranscriptKind::ToolActivity)
    );
}

#[test]
fn provider_failure_is_a_durable_redacted_notice() {
    let input = input(PublicWireFact::Event(
        NormalizedProviderEvent::ProviderFailure {
            classification: ProviderFailureClassification::Authentication,
            message: "Codex authentication failed.".into(),
        },
    ));
    assert_eq!(
        transcript_content(&input.fact),
        Some((
            gent_types::NormalizedTranscriptKind::Notice,
            "Codex authentication failed.".into(),
            false,
        ))
    );
    let batch = batch("daemon-1", &input).unwrap();
    assert!(matches!(
        batch.lifecycle,
        gent_types::NormalizedSessionLifecycle::Event {
            event: NormalizedProviderEvent::ProviderFailure {
                classification: ProviderFailureClassification::Authentication,
                ..
            }
        }
    ));
    assert_eq!(
        batch.transcript.as_ref().map(|item| item.kind),
        Some(gent_types::NormalizedTranscriptKind::Notice)
    );
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
fn structured_tool_and_subagent_lifecycle_preserve_provider_neutral_correlations() {
    let tool = input(PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::ToolActivity {
            activity: ToolActivity {
                tool_use_id: "tool-1".into(),
                tool_name: "mcp__linear_cloud__search_issues".into(),
                phase: ToolPhase::WaitingPermission,
                output_digest: Some("a".repeat(64)),
            },
        },
    ));
    assert!(matches!(
        activity(&tool),
        Some(gent_types::ConversationActivityFact::ToolActivity { activity, .. })
            if activity.tool_use_id == "tool-1"
                && activity.tool_name == "mcp__linear_cloud__search_issues"
                && activity.phase == ToolPhase::WaitingPermission
    ));
    let child = input(PublicWireFact::Event(
        NormalizedProviderEvent::ChildStarted {
            child_id: "child-1".into(),
            parent_tool_use_id: "tool-1".into(),
        },
    ));
    assert!(matches!(
        activity(&child),
        Some(gent_types::ConversationActivityFact::SubagentStarted { child_id, parent_tool_use_id, .. })
            if child_id == "child-1" && parent_tool_use_id == "tool-1"
    ));
    let terminal = input(PublicWireFact::Event(
        NormalizedProviderEvent::ChildTerminal {
            child_id: "child-1".into(),
            phase: WorkPhase::Done,
        },
    ));
    assert!(matches!(
        activity(&terminal),
        Some(gent_types::ConversationActivityFact::WorkPhase {
            kind: gent_types::ActivityWorkKind::Subagent,
            phase: WorkPhase::Done,
            ..
        })
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
fn provider_turn_ids_are_scoped_to_the_gent_owned_chat_turn() {
    let batch = batch(
        "daemon-1",
        &input(PublicWireFact::Event(
            NormalizedProviderEvent::TurnStarted {
                turn_id: "provider-native-turn".into(),
            },
        )),
    )
    .unwrap();
    assert!(matches!(
        batch.lifecycle,
        gent_types::NormalizedSessionLifecycle::Event {
            event: NormalizedProviderEvent::TurnStarted { turn_id }
        } if turn_id == "turn-1"
    ));
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
