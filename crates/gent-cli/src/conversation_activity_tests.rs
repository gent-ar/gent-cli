use gent_protocol::{Negotiated, read_frame, read_json_frame, write_frame, write_json_frame};
use gent_types::{
    CONVERSATION_ACTIVITY_SCHEMA_VERSION, CapabilitySet, ConversationActivity,
    ConversationActivityState, HostEpoch, TurnPhase,
};
use tokio::net::UnixListener;

use super::*;

fn activity() -> ConversationActivity {
    ConversationActivity {
        schema_version: CONVERSATION_ACTIVITY_SCHEMA_VERSION,
        conversation_id: "conversation-1".into(),
        run_id: "run-1".into(),
        host_epoch: HostEpoch(1),
        revision: 2,
        activity_sequence: 2,
        cursor: 9,
        active_turn_id: Some("turn-1".into()),
        root_phase: TurnPhase::Processing,
        state: ConversationActivityState::Thinking,
        pending_decision_id: None,
        work: Vec::new(),
        has_error: false,
    }
}

#[tokio::test]
async fn request_negotiates_and_reads_a_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        assert!(matches!(
            read_frame(&mut stream).await.unwrap(),
            WireFrame::Hello(Hello { capabilities, .. })
                if capabilities.0.iter().any(|item| item == CONVERSATION_ACTIVITY_CAPABILITY)
        ));
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
            read_json_frame::<_, ConversationActivityFrame>(&mut stream).await.unwrap(),
            ConversationActivityFrame::Request { conversation_id, run_id, after_cursor }
                if conversation_id == "conversation-1" && run_id == "run-1" && after_cursor == 4
        ));
        write_json_frame(
            &mut stream,
            &ConversationActivityFrame::Snapshot(activity()),
        )
        .await
        .unwrap();
    });
    assert!(matches!(
        request(Some(directory.path().into()), true, "conversation-1".into(), "run-1".into(), 4)
            .await
            .unwrap(),
        ActivityRead::Snapshot(activity) if activity.cursor == 9
    ));
}

#[tokio::test]
async fn observer_capability_set_rejects_the_authority_gated_read() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = read_frame(&mut stream).await.unwrap();
        write_frame(
            &mut stream,
            &WireFrame::Negotiated(Negotiated {
                protocol: PROTOCOL_MAX,
                capabilities: CapabilitySet(Vec::new()),
            }),
        )
        .await
        .unwrap();
    });
    let error = request(
        Some(directory.path().into()),
        true,
        "conversation-1".into(),
        "run-1".into(),
        0,
    )
    .await
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("authoritative conversation activity")
    );
}

#[test]
fn delta_requires_one_identity_and_strictly_increasing_activity_state() {
    let first = activity();
    let mut second = activity();
    second.cursor = 10;
    second.revision = 3;
    second.activity_sequence = 3;
    assert!(matches!(
        decode(
            ConversationActivityFrame::Delta(vec![first.clone(), second]),
            "conversation-1",
            "run-1",
            8,
        ),
        Ok(ActivityRead::Delta(items)) if items.len() == 2
    ));

    assert!(
        decode(
            ConversationActivityFrame::Delta(vec![first, activity()]),
            "conversation-1",
            "run-1",
            8,
        )
        .is_err()
    );

    let mut wrong_run = activity();
    wrong_run.run_id = "other".into();
    assert!(
        decode(
            ConversationActivityFrame::Delta(vec![wrong_run]),
            "conversation-1",
            "run-1",
            8,
        )
        .is_err()
    );
}

#[test]
fn snapshot_cannot_regress_the_client_cursor() {
    assert!(
        decode(
            ConversationActivityFrame::Snapshot(activity()),
            "conversation-1",
            "run-1",
            10,
        )
        .is_err()
    );
}

#[test]
fn snapshot_requires_the_supported_activity_schema() {
    let mut unsupported = activity();
    unsupported.schema_version += 1;
    let error = decode(
        ConversationActivityFrame::Snapshot(unsupported),
        "conversation-1",
        "run-1",
        0,
    )
    .unwrap_err();
    assert!(error.contains("unsupported conversation activity schema version"));
}
