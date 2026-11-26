use super::*;
use gent_protocol::ConversationStatusFrame;
use gent_types::{ConversationActivityFact, ConversationActivityPage, ConversationActivityScope};

fn scope(cursor: u64) -> ConversationActivityScope {
    ConversationActivityScope {
        conversation_id: "conversation-1".into(),
        run_id: "running-run".into(),
        turn_id: "queued-turn".into(),
        host_epoch: HostEpoch(1),
        cursor,
    }
}

pub(super) fn queued(cursor: u64, message_id: &str) -> ConversationActivityFact {
    ConversationActivityFact::PromptQueued {
        scope: scope(cursor),
        message_id: message_id.into(),
        queue_delivery: None,
        cancelable: None,
    }
}

fn requested(cursor: u64, message_id: &str) -> ConversationActivityFact {
    ConversationActivityFact::PromptSteerRequested {
        scope: scope(cursor),
        message_id: message_id.into(),
        receipt_id: "outstanding".into(),
    }
}

pub(super) async fn reply_status(listener: &UnixListener) {
    let (mut stream, _) = listener.accept().await.unwrap();
    let _ = read_frame(&mut stream).await.unwrap();
    write_frame(
        &mut stream,
        &WireFrame::Negotiated(Negotiated {
            protocol: PROTOCOL_MAX,
            capabilities: CapabilitySet(vec![gent_protocol::CONVERSATION_STATUS_CAPABILITY.into()]),
        }),
    )
    .await
    .unwrap();
    assert!(matches!(
        read_json_frame(&mut stream).await.unwrap(),
        ConversationStatusFrame::Request { conversation_id } if conversation_id == "conversation-1"
    ));
    write_json_frame(
        &mut stream,
        &ConversationStatusFrame::Status(gent_types::ConversationStatus {
            conversation_id: "conversation-1".into(),
            runs: vec![target()],
        }),
    )
    .await
    .unwrap();
}

pub(super) async fn reply_activity(listener: &UnixListener, facts: Vec<ConversationActivityFact>) {
    let (mut stream, _) = listener.accept().await.unwrap();
    let _ = read_frame(&mut stream).await.unwrap();
    write_frame(
        &mut stream,
        &WireFrame::Negotiated(Negotiated {
            protocol: PROTOCOL_MAX,
            capabilities: CapabilitySet(vec![
                gent_protocol::CONVERSATION_ACTIVITY_CAPABILITY.into(),
            ]),
        }),
    )
    .await
    .unwrap();
    assert!(matches!(
        read_json_frame(&mut stream).await.unwrap(),
        gent_protocol::ConversationActivityFrame::Request { run_id, .. } if run_id == "running-run"
    ));
    write_json_frame(
        &mut stream,
        &gent_protocol::ConversationActivityFrame::Facts(ConversationActivityPage {
            facts,
            next_after_cursor: None,
        }),
    )
    .await
    .unwrap();
}

async fn steer(listener: &UnixListener, expected_message: &str, reject: bool) {
    let (mut stream, _) = listener.accept().await.unwrap();
    let _ = read_frame(&mut stream).await.unwrap();
    write_frame(
        &mut stream,
        &WireFrame::Negotiated(Negotiated {
            protocol: PROTOCOL_MAX,
            capabilities: capabilities(),
        }),
    )
    .await
    .unwrap();
    let AgentChatIntentFrame::SteerQueuedPromptForTurn {
        request_id,
        receipt_id,
        conversation_id,
        message_id,
        expected_run_id,
        expected_turn_id,
    } = read_json_frame(&mut stream).await.unwrap()
    else {
        panic!("expected steer intent");
    };
    assert_eq!(message_id, expected_message);
    assert_eq!(expected_run_id.0, "running-run");
    assert_eq!(expected_turn_id, "observed-turn");
    if reject {
        write_frame(
            &mut stream,
            &WireFrame::Error {
                code: "queuedPromptNotSteerable".into(),
                message: "already requested".into(),
            },
        )
        .await
        .unwrap();
    } else {
        write_json_frame(
            &mut stream,
            &AgentChatIntentFrame::QueuedPromptSteered {
                request_id,
                receipt: receipt(receipt_id),
                conversation_id,
                message_id,
            },
        )
        .await
        .unwrap();
    }
}

async fn steer_explicit(
    directory: &tempfile::TempDir,
    message_id: &str,
) -> Result<Vec<AgentChatIntentFrame>, Box<dyn std::error::Error>> {
    super::super::steer(
        Some(directory.path().into()),
        true,
        super::super::SteerArgs {
            conversation_id: "conversation-1".into(),
            message_id: Some(message_id.into()),
        },
    )
    .await
}

#[tokio::test]
async fn cli_steer_does_not_retry_a_stale_target() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    let server = tokio::spawn(async move {
        reply_status(&listener).await;
        reply_activity(&listener, vec![]).await;
        steer(&listener, "queued-message", true).await;
        reply_status(&listener).await;
        reply_activity(&listener, vec![queued(1, "queued-message")]).await;
        listener
    });
    let error = steer_explicit(&directory, "queued-message")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("queuedPromptNotSteerable"));
    let listener = server.await.unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn explicit_steer_rejects_an_outstanding_request() {
    explicit_steer_rejection(
        vec![queued(1, "queued-message"), requested(2, "queued-message")],
        "already outstanding",
    )
    .await;
}

#[tokio::test]
async fn explicit_steer_rejects_unproven_delivery() {
    explicit_steer_rejection(
        vec![
            queued(1, "queued-message"),
            ConversationActivityFact::PromptSteerRejected {
                scope: scope(2),
                message_id: "queued-message".into(),
                receipt_id: "receipt".into(),
                reason: "deliveryUnprovable".into(),
            },
        ],
        "could not be proven",
    )
    .await;
}

async fn explicit_steer_rejection(facts: Vec<ConversationActivityFact>, expected: &str) {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    let server = tokio::spawn(async move {
        reply_status(&listener).await;
        reply_activity(&listener, facts).await;
        listener
    });
    let error = steer_explicit(&directory, "queued-message")
        .await
        .unwrap_err();
    assert!(error.to_string().contains(expected));
    let listener = server.await.unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn duplicate_rejection_continues_with_the_remaining_queue() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    let server = tokio::spawn(async move {
        reply_status(&listener).await;
        reply_activity(
            &listener,
            vec![queued(1, "A"), queued(2, "B"), queued(3, "C")],
        )
        .await;
        steer(&listener, "A", false).await;
        steer(&listener, "B", true).await;
        reply_status(&listener).await;
        reply_activity(&listener, vec![queued(2, "B"), requested(4, "B")]).await;
        steer(&listener, "C", false).await;
    });
    let replies = super::super::steer(
        Some(directory.path().into()),
        true,
        super::super::SteerArgs {
            conversation_id: "conversation-1".into(),
            message_id: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(replies.len(), 2);
    server.await.unwrap();
}
