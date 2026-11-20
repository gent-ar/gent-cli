use std::collections::BTreeMap;

use gent_types::{NormalizedLifecycleSignal, NormalizedProviderEvent, WorkPhase};
use serde_json::Value;

use crate::public_protocol::PublicWireFact;

pub(super) fn child_thread<'a>(
    frame: &'a Value,
    children: &BTreeMap<String, String>,
    root_thread: Option<&str>,
) -> Option<&'a str> {
    let thread_id = frame.pointer("/params/threadId")?.as_str()?;
    (children.contains_key(thread_id) || root_thread.is_some_and(|root| root != thread_id))
        .then_some(thread_id)
}

pub(super) fn attribute_to_child(
    facts: Vec<PublicWireFact>,
    parent: Option<&str>,
) -> Vec<PublicWireFact> {
    facts
        .into_iter()
        .filter_map(|fact| match fact {
            PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { mut activity }) => {
                activity.parent_tool_use_id = activity
                    .parent_tool_use_id
                    .or_else(|| parent.map(str::to_owned));
                Some(PublicWireFact::Lifecycle(
                    NormalizedLifecycleSignal::ToolActivity { activity },
                ))
            }
            PublicWireFact::Event(NormalizedProviderEvent::Output { text, is_partial }) => parent
                .map(|parent| {
                    PublicWireFact::Event(NormalizedProviderEvent::ToolOutputDelta {
                        tool_use_id: parent.into(),
                        text,
                        is_partial,
                    })
                }),
            PublicWireFact::Event(
                NormalizedProviderEvent::TurnStarted { .. }
                | NormalizedProviderEvent::TurnEnded { .. }
                | NormalizedProviderEvent::ProviderFailure { .. }
                | NormalizedProviderEvent::Thinking { .. }
                | NormalizedProviderEvent::ContextUsage { .. }
                | NormalizedProviderEvent::TokenUsage { .. },
            )
            | PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::RootActivity { .. }
                | NormalizedLifecycleSignal::RootPhase { .. },
            ) => None,
            fact => Some(fact),
        })
        .collect()
}

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
            .and_then(Value::as_str)?
        {
            "completed" => WorkPhase::Done,
            "interrupted" => WorkPhase::Interrupted,
            "failed" => WorkPhase::Failed,
            _ => return None,
        },
        "thread/status/changed" => match frame
            .pointer("/params/status/type")
            .and_then(Value::as_str)?
        {
            "active" => WorkPhase::Running,
            "systemError" => WorkPhase::Failed,
            _ => return None,
        },
        "thread/closed" => WorkPhase::Interrupted,
        _ => return None,
    };
    Some((child_id.into(), phase))
}

pub(super) fn reported_child_terminals(frame: &Value) -> Vec<(String, WorkPhase)> {
    let Some(item) = frame
        .pointer("/params/item")
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("collabAgentToolCall"))
    else {
        return Vec::new();
    };
    item.get("agentsStates")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(child_id, state)| {
            let phase = match state.get("status").and_then(Value::as_str)? {
                "completed" => WorkPhase::Done,
                "errored" | "notFound" => WorkPhase::Failed,
                "interrupted" | "shutdown" => WorkPhase::Interrupted,
                _ => return None,
            };
            Some((child_id.clone(), phase))
        })
        .collect()
}

pub(super) fn command_completion_fallback(
    tool_output_item_ids: &mut std::collections::BTreeSet<String>,
    frame: &Value,
) -> Option<PublicWireFact> {
    if method(frame) != Some("item/completed") {
        return None;
    }
    let item = frame.pointer("/params/item")?;
    if item.get("type").and_then(Value::as_str) != Some("commandExecution") {
        return None;
    }
    let id = item
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())?;
    if tool_output_item_ids.contains(id) {
        return None;
    }
    let text = item
        .get("aggregatedOutput")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())?;
    tool_output_item_ids.insert(id.into());
    Some(PublicWireFact::Event(
        NormalizedProviderEvent::ToolOutputDelta {
            tool_use_id: id.into(),
            text: text.into(),
            is_partial: false,
        },
    ))
}

pub(super) fn root_session_idle(frame: &Value, root_thread: Option<&str>) -> Option<bool> {
    let params = frame
        .get("params")
        .filter(|_| method(frame) == Some("thread/status/changed"))?;
    if params.get("threadId").and_then(Value::as_str) != Some(root_thread?) {
        return None;
    }
    Some(params.pointer("/status/type").and_then(Value::as_str)? != "active")
}

pub(super) fn rate_limit_resets_at(frame: &Value) -> Option<u64> {
    let limits = frame
        .pointer("/params/rateLimits")
        .filter(|_| method(frame) == Some("account/rateLimits/updated"))?;
    let exhausted = |window: &str| {
        limits
            .pointer(&format!("/{window}/usedPercent"))
            .and_then(Value::as_u64)
            .is_some_and(|used| used >= 100)
    };
    let window = if exhausted("secondary") && !exhausted("primary") {
        "secondary"
    } else {
        "primary"
    };
    limits
        .pointer(&format!("/{window}/resetsAt"))
        .and_then(Value::as_u64)
}

pub(super) fn attach_resets_at(facts: &mut [PublicWireFact], known: Option<u64>) {
    for fact in facts {
        if let PublicWireFact::Event(NormalizedProviderEvent::ProviderFailure {
            classification: gent_types::ProviderFailureClassification::RateLimited,
            resets_at,
            ..
        }) = fact
        {
            *resets_at = known;
        }
    }
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use gent_types::WorkPhase;
    use serde_json::json;

    use super::{child_phase, reported_child_terminals};

    #[test]
    fn a_child_ends_only_on_statuses_the_codex_schema_defines() {
        let children = BTreeMap::from([("child".to_owned(), "parent".to_owned())]);
        let phase = |frame| child_phase(&frame, &children).map(|(_, phase)| phase);
        assert_eq!(
            phase(
                json!({"method":"turn/completed","params":{"threadId":"child","turn":{"status":"failed"}}})
            ),
            Some(WorkPhase::Failed)
        );
        assert_eq!(
            phase(
                json!({"method":"turn/completed","params":{"threadId":"child","status":"completed"}})
            ),
            None
        );
        assert_eq!(
            phase(
                json!({"method":"turn/completed","params":{"threadId":"child","turn":{"status":"cancelled"}}})
            ),
            None
        );
        assert_eq!(
            phase(
                json!({"method":"thread/status/changed","params":{"threadId":"child","status":{"type":"working"}}})
            ),
            None
        );
        assert_eq!(
            phase(json!({"method":"thread/closed","params":{"threadId":"child"}})),
            Some(WorkPhase::Interrupted)
        );
        assert_eq!(
            phase(json!({"method":"thread/closed","params":{"threadId":"root"}})),
            None
        );
    }

    #[test]
    fn collab_agent_states_report_each_child_terminal_status() {
        let mut reported = reported_child_terminals(&json!({
            "method": "item/completed",
            "params": {"item": {
                "type": "collabAgentToolCall",
                "id": "call",
                "receiverThreadIds": ["a", "b", "c", "d"],
                "agentsStates": {
                    "a": {"status": "completed", "message": null},
                    "b": {"status": "errored", "message": "x"},
                    "c": {"status": "shutdown", "message": null},
                    "d": {"status": "running", "message": null}
                }
            }}
        }));
        reported.sort_by(|left, right| left.0.cmp(&right.0));
        assert_eq!(
            reported,
            [
                ("a".to_owned(), WorkPhase::Done),
                ("b".to_owned(), WorkPhase::Failed),
                ("c".to_owned(), WorkPhase::Interrupted),
            ]
        );
    }
}
