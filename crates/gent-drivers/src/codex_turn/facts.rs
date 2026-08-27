use std::collections::BTreeMap;

use gent_types::{NormalizedProviderEvent, WorkPhase};
use serde_json::Value;

use crate::public_protocol::PublicWireFact;

pub(super) fn child_terminal(
    frame: &Value,
    children: &BTreeMap<String, String>,
) -> Option<(String, WorkPhase)> {
    let (child_id, phase) = child_phase(frame, children)?;
    matches!(
        phase,
        WorkPhase::Done | WorkPhase::Failed | WorkPhase::Interrupted
    )
    .then_some((child_id, phase))
}

pub(super) fn child_phase(
    frame: &Value,
    children: &BTreeMap<String, String>,
) -> Option<(String, WorkPhase)> {
    let method = method(frame)?;
    let child_id = frame.pointer("/params/threadId").and_then(Value::as_str)?;
    if !children.contains_key(child_id) {
        return None;
    }
    let phase = match method {
        "turn/started" => WorkPhase::Running,
        "turn/completed" => match frame
            .pointer("/params/turn/status")
            .and_then(Value::as_str)
            .or_else(|| frame.pointer("/params/status").and_then(Value::as_str))?
        {
            "completed" => WorkPhase::Done,
            "interrupted" => WorkPhase::Interrupted,
            "failed" => WorkPhase::Failed,
            "cancelled" | "canceled" | "aborted" | "timedOut" | "timed_out" | "timeout" => {
                WorkPhase::Interrupted
            }
            _ => return None,
        },
        "turn/failed" => WorkPhase::Failed,
        "turn/aborted" => WorkPhase::Interrupted,
        "thread/status/changed" => match frame
            .pointer("/params/status/type")
            .and_then(Value::as_str)?
        {
            "pendingInit" | "notLoaded" | "pending" | "queued" => WorkPhase::Pending,
            "active" | "working" | "running" => WorkPhase::Running,
            "systemError" => WorkPhase::Failed,
            "cancelled" | "canceled" | "aborted" | "timedOut" => WorkPhase::Interrupted,
            _ => return None,
        },
        _ => return None,
    };
    Some((child_id.into(), phase))
}

pub(super) fn method(frame: &Value) -> Option<&str> {
    frame.get("method")?.as_str()
}

pub(super) fn is_empty_turn_completion(frame: &Value) -> bool {
    method(frame) == Some("turn/completed")
        && frame
            .get("params")
            .and_then(Value::as_object)
            .is_some_and(|params| params.is_empty())
}

pub(super) fn root_terminal_fact(fact: &PublicWireFact) -> bool {
    matches!(
        fact,
        PublicWireFact::Event(NormalizedProviderEvent::TurnEnded { .. })
            | PublicWireFact::Event(NormalizedProviderEvent::ProviderFailure { .. })
            | PublicWireFact::Lifecycle(_)
    )
}
