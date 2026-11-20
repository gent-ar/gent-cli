use serde_json::{Value, json};

use super::WrittenMessages;
use crate::claude_runner::ClaudeRunnerEffect;
use crate::claude_runner::input::message_uuid;

fn lifecycle(
    messages: &mut WrittenMessages,
    message_id: &str,
    state: &str,
) -> Vec<ClaudeRunnerEffect> {
    messages.lifecycle(Some(&Value::String(message_uuid(message_id))), state)
}

fn init() -> Value {
    json!({"type":"system","subtype":"init","model":"claude-haiku-4-5-20251001"})
}

fn result(message_ids: &[&str], continuation: bool) -> Value {
    let uuids = message_ids
        .iter()
        .map(|id| message_uuid(id))
        .collect::<Vec<_>>();
    let mut frame = json!({"type":"result","subtype":"success","user_message_uuids":uuids});
    if continuation {
        frame["origin"] = json!({"kind":"task-notification"});
    }
    frame
}

fn turn_result(message_ids: &[&str], continuation: bool) -> ClaudeRunnerEffect {
    ClaudeRunnerEffect::TurnResult {
        message_ids: message_ids.iter().map(|id| (*id).to_owned()).collect(),
        continuation,
    }
}

#[test]
fn a_prompt_starts_on_its_lifecycle_and_is_settled_by_the_result_that_lists_it() {
    let mut messages = WrittenMessages::default();
    messages.written("prompt-1");
    assert_eq!(lifecycle(&mut messages, "prompt-1", "queued"), []);
    assert_eq!(
        lifecycle(&mut messages, "prompt-1", "started"),
        [ClaudeRunnerEffect::MessageStarted {
            message_id: "prompt-1".into()
        }]
    );
    assert_eq!(lifecycle(&mut messages, "prompt-1", "started"), []);
    assert_eq!(messages.turn_started(&init()), []);
    assert_eq!(
        messages.result(&result(&["prompt-1"], false)),
        turn_result(&["prompt-1"], false)
    );
    assert_eq!(lifecycle(&mut messages, "prompt-1", "completed"), []);
    assert_eq!(
        messages.turn_started(&init()),
        [ClaudeRunnerEffect::ContinuationStarted]
    );
}

#[test]
fn a_prompt_written_as_a_continuation_starts_is_folded_into_that_continuation_result() {
    let mut messages = WrittenMessages::default();
    messages.written("prompt-2");
    assert_eq!(lifecycle(&mut messages, "prompt-2", "queued"), []);
    assert_eq!(
        messages.turn_started(&init()),
        [ClaudeRunnerEffect::ContinuationStarted]
    );
    assert_eq!(
        lifecycle(&mut messages, "prompt-2", "started"),
        [ClaudeRunnerEffect::MessageStarted {
            message_id: "prompt-2".into()
        }]
    );
    assert_eq!(
        messages.result(&result(&["prompt-2"], true)),
        turn_result(&["prompt-2"], true)
    );
}

#[test]
fn a_continuation_result_without_user_messages_owns_no_prompt() {
    let mut messages = WrittenMessages::default();
    messages.written("prompt-3");
    assert_eq!(
        messages.result(&json!({"type":"result","subtype":"success","user_message_uuids":null,"origin":{"kind":"task-notification"}})),
        turn_result(&[], true)
    );
    assert_eq!(
        lifecycle(&mut messages, "prompt-3", "started"),
        [ClaudeRunnerEffect::MessageStarted {
            message_id: "prompt-3".into()
        }]
    );
}

#[test]
fn a_queued_message_cancelled_before_it_started_is_reported_once() {
    let mut messages = WrittenMessages::default();
    messages.written("prompt-4");
    messages.written("prompt-5");
    assert_eq!(
        lifecycle(&mut messages, "prompt-4", "started"),
        [ClaudeRunnerEffect::MessageStarted {
            message_id: "prompt-4".into()
        }]
    );
    assert_eq!(lifecycle(&mut messages, "prompt-4", "cancelled"), []);
    assert_eq!(
        lifecycle(&mut messages, "prompt-5", "cancelled"),
        [ClaudeRunnerEffect::MessageCancelled {
            message_id: "prompt-5".into()
        }]
    );
    assert_eq!(lifecycle(&mut messages, "prompt-5", "cancelled"), []);
    assert_eq!(
        messages.result(&result(&[], false)),
        turn_result(&["prompt-4"], false)
    );
}

#[test]
fn a_prompt_written_again_after_claude_cancelled_it_carries_a_fresh_uuid() {
    let mut messages = WrittenMessages::default();
    messages.written("prompt-6");
    assert_eq!(
        lifecycle(&mut messages, "prompt-6", "cancelled"),
        [ClaudeRunnerEffect::MessageCancelled {
            message_id: "prompt-6".into()
        }]
    );
    let again = messages.identity("prompt-6");
    assert_ne!(message_uuid(&again), message_uuid("prompt-6"));
    messages.written("prompt-6");
    assert_eq!(
        messages.lifecycle(Some(&Value::String(message_uuid(&again))), "started"),
        [ClaudeRunnerEffect::MessageStarted {
            message_id: "prompt-6".into()
        }]
    );
}

#[test]
fn a_written_prompt_is_withdrawn_once_by_its_uuid_and_never_after_claude_started_it() {
    let mut messages = WrittenMessages::default();
    messages.written("queued");
    messages.written("taken");
    assert_eq!(messages.withdraw("absent"), None);
    assert_eq!(messages.withdraw("queued"), Some(message_uuid("queued")));
    assert_eq!(messages.withdraw("queued"), None);
    lifecycle(&mut messages, "taken", "started");
    assert_eq!(messages.withdraw("taken"), None);
    assert_eq!(
        lifecycle(&mut messages, "queued", "cancelled"),
        [ClaudeRunnerEffect::MessageCancelled {
            message_id: "queued".into()
        }]
    );
}

#[test]
fn the_withdraw_frame_is_claudes_cancel_async_message_for_that_uuid() {
    let frame: Value =
        serde_json::from_slice(&crate::claude_runner::input::withdraw_frame("u-1")).unwrap();
    assert_eq!(
        frame,
        json!({"type":"control_request","request_id":"gent-withdraw-u-1",
               "request":{"subtype":"cancel_async_message","message_uuid":"u-1"}})
    );
}
