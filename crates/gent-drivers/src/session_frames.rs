//! Structural validation for provider lifecycle frames.

use serde_json::Value;

use crate::normalize::normalize_lifecycle;

pub(super) fn non_empty<'a>(frame: &'a Value, field: &str) -> Option<&'a str> {
    frame.get(field)?.as_str().filter(|value| !value.is_empty())
}

pub(super) fn known_lifecycle_frame(kind: &str) -> bool {
    matches!(
        kind,
        "turn_started"
            | "turn_ended"
            | "child_started"
            | "child_terminal"
            | "command_terminal"
            | "decision_settled"
            | "root_phase"
            | "root_activity"
            | "child_phase"
            | "command_phase"
            | "tool_activity"
            | "decision_requested"
    )
}

pub(super) fn valid_lifecycle_frame(kind: &str, frame: &Value) -> bool {
    match kind {
        "turn_started" | "turn_ended" => non_empty(frame, "turn_id").is_some(),
        "child_started" => {
            non_empty(frame, "child_id").is_some()
                && non_empty(frame, "parent_tool_use_id").is_some()
        }
        "child_terminal" | "child_phase" => {
            non_empty(frame, "child_id").is_some() && non_empty(frame, "phase").is_some()
        }
        "command_terminal" | "command_phase" => {
            non_empty(frame, "command_id").is_some() && non_empty(frame, "phase").is_some()
        }
        "tool_activity" => {
            non_empty(frame, "tool_use_id").is_some()
                && non_empty(frame, "tool_name").is_some()
                && non_empty(frame, "phase").is_some()
                && normalize_lifecycle(frame).is_some()
        }
        "decision_settled" => non_empty(frame, "decision_id").is_some(),
        "root_phase" | "root_activity" => normalize_lifecycle(frame).is_some(),
        "decision_requested" => true,
        _ => false,
    }
}
