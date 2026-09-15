use gent_types::{
    NormalizedLifecycleSignal, NormalizedProviderEvent, ProviderFailureClassification,
    RootActivity, TurnPhase,
};
use serde_json::Value;

use super::{
    PublicWireFact,
    support::{diagnostic, string},
};

pub(super) fn started(frame: &Value) -> Vec<PublicWireFact> {
    turn_id(frame).map_or_else(
        || diagnostic("malformedCodexTurn"),
        |turn_id| {
            vec![PublicWireFact::Event(
                NormalizedProviderEvent::TurnStarted {
                    turn_id: turn_id.into(),
                },
            )]
        },
    )
}

pub(super) fn completed(frame: &Value) -> Vec<PublicWireFact> {
    let turn = frame.pointer("/params/turn");
    let phase = match turn
        .and_then(|value| string(value, "status"))
        .or_else(|| frame.pointer("/params/status").and_then(Value::as_str))
    {
        Some("completed") => TurnPhase::Ready,
        Some("interrupted") => TurnPhase::Interrupted,
        Some("failed") => TurnPhase::Failed,
        _ => return diagnostic("malformedCodexTurn"),
    };
    terminal(turn_id(frame), phase.clone(), failure(frame, &phase))
}

fn terminal(
    turn_id: Option<&str>,
    phase: TurnPhase,
    failure: Option<(ProviderFailureClassification, &'static str)>,
) -> Vec<PublicWireFact> {
    let Some(turn_id) = turn_id else {
        return diagnostic("malformedCodexTurn");
    };
    let mut facts = Vec::new();
    if let Some((classification, message)) = failure {
        facts.push(PublicWireFact::Event(
            NormalizedProviderEvent::ProviderFailure {
                classification,
                message: message.into(),
            },
        ));
    }
    facts.extend([
        PublicWireFact::Event(NormalizedProviderEvent::TurnEnded {
            turn_id: turn_id.into(),
        }),
        PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootActivity {
            activity: RootActivity::Idle,
        }),
        PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootPhase { phase }),
    ]);
    facts
}

fn failure(
    frame: &Value,
    phase: &TurnPhase,
) -> Option<(ProviderFailureClassification, &'static str)> {
    if *phase != TurnPhase::Failed {
        return None;
    }
    Some(super::failure::classify(
        frame.pointer("/params/turn/error"),
        "Codex ended the turn with an error.",
    ))
}

fn turn_id(frame: &Value) -> Option<&str> {
    frame
        .pointer("/params/turn/id")
        .or_else(|| frame.pointer("/params/turnId"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
}
