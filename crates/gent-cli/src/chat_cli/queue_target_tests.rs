use gent_types::{ConversationRunStatus, ConversationStatus};

use super::{explicit_steer_ids, steer_target};

fn status() -> ConversationStatus {
    ConversationStatus {
        conversation_id: "conversation".into(),
        runs: vec![ConversationRunStatus {
            run_id: "running-run".into(),
            parent_run_id: None,
            provider: "codex".into(),
            active_turn_id: Some("observed-turn".into()),
            live_status: None,
        }],
    }
}

#[test]
fn target_is_the_observed_active_run_not_the_newest_run_or_queued_prompt() {
    let mut status = status();
    let observed = steer_target(&status).unwrap();
    let mut next = observed.clone();
    next.run_id = "new-selection-run".into();
    next.provider = "claurst".into();
    next.active_turn_id = None;
    status.runs.push(next);
    assert_eq!(steer_target(&status), Some(observed.clone()));
    status.runs[0].active_turn_id = Some("later-turn".into());
    assert_eq!(observed.active_turn_id.as_deref(), Some("observed-turn"));
}

#[test]
fn missing_empty_or_ambiguous_targets_are_not_steerable() {
    let mut status = status();
    status.runs.push(status.runs[0].clone());
    assert!(steer_target(&status).is_none());
    status.runs.pop();
    status.runs[0].active_turn_id = Some(String::new());
    assert!(steer_target(&status).is_none());
    status.runs[0].active_turn_id = None;
    assert!(steer_target(&status).is_none());
    status.runs.clear();
    assert!(steer_target(&status).is_none());
}

#[test]
fn steer_selection_skips_outstanding_and_uncertain_but_cancel_keeps_them() {
    let queue = ["A", "B", "C", "uncertain"]
        .into_iter()
        .enumerate()
        .map(|(cursor, message)| crate::prompt_queue::QueuedPrompt {
            message_id: message.into(),
            run_id: "run".into(),
            turn_id: format!("turn-{message}"),
            cursor: cursor as u64,
            delivery_unconfirmed: message == "uncertain",
            steer_receipt_id: (message == "B").then(|| "outstanding".into()),
        })
        .collect::<Vec<_>>();
    assert_eq!(super::select_ids(queue.clone(), true).unwrap(), ["A", "C"]);
    assert_eq!(
        super::select_ids(queue, false).unwrap(),
        ["A", "B", "C", "uncertain"]
    );
}

#[test]
fn explicit_steer_explains_outstanding_and_unconfirmed_messages() {
    let queue = [
        ("requested", false, Some("receipt")),
        ("unconfirmed", true, None),
    ]
    .into_iter()
    .map(
        |(message_id, delivery_unconfirmed, steer_receipt_id)| crate::prompt_queue::QueuedPrompt {
            message_id: message_id.into(),
            run_id: "run".into(),
            turn_id: "turn".into(),
            cursor: 1,
            delivery_unconfirmed,
            steer_receipt_id: steer_receipt_id.map(str::to_owned),
        },
    )
    .collect::<Vec<_>>();
    assert!(
        explicit_steer_ids(&queue, "requested".into())
            .unwrap_err()
            .to_string()
            .contains("already outstanding")
    );
    assert!(
        explicit_steer_ids(&queue, "unconfirmed".into())
            .unwrap_err()
            .to_string()
            .contains("could not be proven")
    );
}
