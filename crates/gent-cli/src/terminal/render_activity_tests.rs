use gent_types::{ConversationActivityScope, HostEpoch};

use super::timeline_lines;

#[test]
fn activity_timeline_uses_durable_cursor_order() {
    let lines = timeline_lines(&[started(9), started(3)]);
    assert!(lines[0].to_string().starts_with("#3"));
    assert!(lines[1].to_string().starts_with("#9"));
}

fn started(cursor: u64) -> gent_types::ConversationActivityFact {
    gent_types::ConversationActivityFact::TurnStarted {
        scope: ConversationActivityScope {
            conversation_id: "conversation".into(),
            run_id: "run".into(),
            turn_id: "turn".into(),
            host_epoch: HostEpoch(1),
            cursor,
        },
    }
}
