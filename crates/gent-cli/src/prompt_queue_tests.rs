use gent_types::{ConversationActivityFact, ConversationActivityScope, HostEpoch};

use super::queued_prompts;

fn scope(cursor: u64, turn: &str) -> ConversationActivityScope {
    ConversationActivityScope {
        conversation_id: "conversation-1".into(),
        run_id: "run-1".into(),
        turn_id: turn.into(),
        host_epoch: HostEpoch(1),
        cursor,
    }
}

fn queued(cursor: u64, message: &str) -> ConversationActivityFact {
    ConversationActivityFact::PromptQueued {
        scope: scope(cursor, &format!("turn-{message}")),
        message_id: message.into(),
        queue_delivery: None,
        cancelable: None,
    }
}

#[test]
fn only_prompts_still_waiting_in_the_queue_are_listed_oldest_first() {
    let facts = vec![
        queued(3, "second"),
        queued(1, "first"),
        queued(4, "released"),
        queued(5, "canceled"),
        queued(6, "steered"),
        ConversationActivityFact::PromptReleased {
            scope: scope(7, "turn-released"),
            message_id: "released".into(),
        },
        ConversationActivityFact::PromptCanceled {
            scope: scope(8, "turn-canceled"),
            message_id: "canceled".into(),
        },
        ConversationActivityFact::PromptSteered {
            scope: scope(9, "turn-active"),
            message_id: "steered".into(),
            receipt_id: "receipt".into(),
            transcript_cursor: 2,
        },
    ];
    let ids = queued_prompts(&facts)
        .into_iter()
        .map(|prompt| prompt.message_id)
        .collect::<Vec<_>>();
    assert_eq!(ids, ["first", "second"]);
}

#[test]
fn an_empty_activity_log_has_no_queue() {
    assert!(queued_prompts(&[]).is_empty());
}

#[test]
fn an_unconfirmed_steer_remains_available_to_remove() {
    let mut facts = vec![
        queued(1, "unconfirmed"),
        queued(2, "waiting"),
        ConversationActivityFact::PromptSteerRejected {
            scope: scope(3, "turn-active"),
            message_id: "unconfirmed".into(),
            receipt_id: "steer-receipt".into(),
            reason: "deliveryUnprovable".into(),
        },
    ];
    let queue = queued_prompts(&facts);
    assert_eq!(queue.len(), 2);
    assert!(queue[0].delivery_unconfirmed);
    assert!(!queue[0].can_steer());
    assert!(!queue[1].delivery_unconfirmed);
    facts.push(ConversationActivityFact::PromptCanceled {
        scope: scope(4, "turn-unconfirmed"),
        message_id: "unconfirmed".into(),
    });
    assert_eq!(queued_prompts(&facts), queue[1..]);
}

fn requested(cursor: u64, message: &str, receipt: &str) -> ConversationActivityFact {
    ConversationActivityFact::PromptSteerRequested {
        scope: scope(cursor, "turn-active"),
        message_id: message.into(),
        receipt_id: receipt.into(),
    }
}

fn rejected(cursor: u64, receipt: &str) -> ConversationActivityFact {
    ConversationActivityFact::PromptSteerRejected {
        scope: scope(cursor, "turn-active"),
        message_id: "B".into(),
        receipt_id: receipt.into(),
        reason: "deliveryRejected".into(),
    }
}

#[test]
fn outstanding_receipt_skips_only_b_and_preserves_a_then_c() {
    let mut facts = vec![
        queued(3, "C"),
        queued(1, "A"),
        queued(2, "B"),
        requested(4, "B", "old"),
        rejected(5, "old"),
        requested(6, "B", "current"),
        rejected(7, "old"),
    ];
    facts.reverse();
    let queue = queued_prompts(&facts);
    assert_eq!(
        queue
            .iter()
            .map(|prompt| prompt.message_id.as_str())
            .collect::<Vec<_>>(),
        ["A", "B", "C"]
    );
    assert_eq!(queue[1].steer_receipt_id.as_deref(), Some("current"));
    assert_eq!(
        queue
            .iter()
            .filter(|prompt| prompt.can_steer())
            .map(|prompt| prompt.message_id.as_str())
            .collect::<Vec<_>>(),
        ["A", "C"]
    );
    facts.push(rejected(8, "current"));
    assert!(
        queued_prompts(&facts)
            .iter()
            .all(|prompt| prompt.can_steer())
    );
}

#[test]
fn outstanding_receipts_are_distinct_from_unproven_delivery() {
    let facts = vec![
        queued(1, "requested"),
        queued(2, "unconfirmed"),
        requested(3, "requested", "receipt"),
        ConversationActivityFact::PromptSteerRejected {
            scope: scope(4, "turn-active"),
            message_id: "unconfirmed".into(),
            receipt_id: "unknown".into(),
            reason: "deliveryUnprovable".into(),
        },
    ];
    let queue = queued_prompts(&facts);
    assert_eq!(queue[0].steer_receipt_id.as_deref(), Some("receipt"));
    assert!(!queue[0].delivery_unconfirmed);
    assert!(queue[1].steer_receipt_id.is_none());
    assert!(queue[1].delivery_unconfirmed);
}

#[test]
fn terminal_queue_facts_remove_outstanding_requests() {
    for terminal in [
        ConversationActivityFact::PromptReleased {
            scope: scope(3, "turn-B"),
            message_id: "B".into(),
        },
        ConversationActivityFact::PromptCanceled {
            scope: scope(3, "turn-B"),
            message_id: "B".into(),
        },
        ConversationActivityFact::PromptSteered {
            scope: scope(3, "turn-active"),
            message_id: "B".into(),
            receipt_id: "current".into(),
            transcript_cursor: 4,
        },
    ] {
        assert!(
            queued_prompts(&[queued(1, "B"), requested(2, "B", "current"), terminal]).is_empty()
        );
    }
}
