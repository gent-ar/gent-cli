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
