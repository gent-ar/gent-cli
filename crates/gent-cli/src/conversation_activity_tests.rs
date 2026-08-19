use gent_protocol::{Negotiated, read_frame, read_json_frame, write_frame, write_json_frame};
use gent_types::{
    CapabilitySet, ConversationActivityFact, ConversationActivityPage, ConversationActivityScope,
    HostEpoch,
};
use tokio::net::UnixListener;

use super::*;

fn fact(cursor: u64) -> ConversationActivityFact {
    ConversationActivityFact::TurnStarted {
        scope: ConversationActivityScope {
            conversation_id: "conversation-1".into(),
            run_id: "run-1".into(),
            turn_id: "turn-1".into(),
            host_epoch: HostEpoch(1),
            cursor,
        },
    }
}

#[tokio::test]
async fn request_negotiates_and_reads_a_fact_page() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = read_frame(&mut stream).await.unwrap();
        write_frame(
            &mut stream,
            &WireFrame::Negotiated(Negotiated {
                protocol: PROTOCOL_MAX,
                capabilities: CapabilitySet(vec![CONVERSATION_ACTIVITY_CAPABILITY.into()]),
            }),
        )
        .await
        .unwrap();
        assert!(matches!(
            read_json_frame::<_, ConversationActivityFrame>(&mut stream)
                .await
                .unwrap(),
            ConversationActivityFrame::Request {
                after_cursor: 4,
                ..
            }
        ));
        write_json_frame(
            &mut stream,
            &ConversationActivityFrame::Facts(ConversationActivityPage {
                facts: vec![fact(9)],
                next_after_cursor: None,
            }),
        )
        .await
        .unwrap();
    });
    assert!(matches!(
        request(Some(directory.path().into()), true, "conversation-1".into(), "run-1".into(), 4)
            .await
            .unwrap(),
        ActivityRead(ConversationActivityPage { facts, next_after_cursor: None }) if facts == vec![fact(9)]
    ));
}

#[test]
fn page_requires_one_identity_and_strictly_increasing_cursors() {
    let page = ConversationActivityPage {
        facts: vec![fact(9), fact(10)],
        next_after_cursor: None,
    };
    assert!(
        decode(
            ConversationActivityFrame::Facts(page),
            "conversation-1",
            "run-1",
            8
        )
        .is_ok()
    );
    assert!(
        decode(
            ConversationActivityFrame::Facts(ConversationActivityPage {
                facts: vec![fact(9), fact(9)],
                next_after_cursor: None,
            }),
            "conversation-1",
            "run-1",
            8
        )
        .is_err()
    );
}

#[test]
fn next_cursor_must_name_the_last_fact() {
    assert!(
        decode(
            ConversationActivityFrame::Facts(ConversationActivityPage {
                facts: vec![fact(9)],
                next_after_cursor: Some(8),
            }),
            "conversation-1",
            "run-1",
            0
        )
        .is_err()
    );
}
