use std::time::Duration;

use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AGENT_CHAT_PROJECTION_CAPABILITY, AgentChatIntentFrame,
    AgentChatProjectionDelta, AgentChatProjectionFrame, Hello, HistoricalTranscriptEntry,
    ProjectionCursor, WireFrame, read_frame, read_json_frame, write_frame, write_json_frame,
};
use gent_runtime::catalog::{RuntimeCapabilityFeature, RuntimeCapabilityProfile};
use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, CapabilitySet, NormalizedTranscriptKind, PROTOCOL_MAX,
    PROTOCOL_MIN, ReceiptId,
};
use tokio::io::{DuplexStream, duplex};

use crate::{CompatibilityAssessment, RuntimeFacade, api::RuntimeApi, build_runtime};

#[tokio::test]
async fn snapshot_is_the_latest_window_and_follow_resumes_exactly_after_it() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = build_runtime(
        directory.path(),
        &RuntimeCapabilityProfile::new([
            RuntimeCapabilityFeature::AgentChat,
            RuntimeCapabilityFeature::AgentChatProjection,
        ]),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let (conversation_id, run_id) = create(&runtime);
    import(&runtime, &conversation_id, &run_id, 0..150);

    let mut snapshot_client = connect(&runtime).await;
    write_json_frame(
        &mut snapshot_client,
        &AgentChatProjectionFrame::ConversationSnapshotRequest {
            request_id: "snapshot-1".into(),
            conversation_id: conversation_id.0.clone(),
            transcript_limit: 100,
            activity_limit: 100,
        },
    )
    .await
    .unwrap();
    let AgentChatProjectionFrame::ConversationSnapshot { snapshot, .. } =
        read_json_frame(&mut snapshot_client).await.unwrap()
    else {
        panic!("snapshot request must return a snapshot");
    };
    let texts = snapshot
        .transcript
        .iter()
        .map(|event| event.text.clone())
        .collect::<Vec<_>>();
    assert_eq!(texts.len(), 100);
    assert_eq!(texts.first().unwrap(), "entry 50");
    assert_eq!(texts.last().unwrap(), "entry 149");

    let mut replay = follow(&runtime, &conversation_id, ProjectionCursor { value: 0 }).await;
    let replayed = deltas(&mut replay, 150, 0).await;
    assert_eq!(replayed.last().unwrap().0, snapshot.cursor.value);

    let mut live = follow(&runtime, &conversation_id, snapshot.cursor.clone()).await;
    import(&runtime, &conversation_id, &run_id, 150..300);
    let streamed = tokio::time::timeout(
        Duration::from_secs(3),
        deltas(&mut live, 150, snapshot.cursor.value),
    )
    .await
    .expect("every new projection event streams without per-event polling delay");
    assert_eq!(
        streamed
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>(),
        (150..300)
            .map(|index| format!("entry {index}"))
            .collect::<Vec<_>>()
    );
}

fn create(runtime: &RuntimeFacade) -> (AgentChatConversationId, AgentChatRunId) {
    let created = runtime
        .agent_chat_intent(AgentChatIntentFrame::CreateConversation {
            request_id: AgentChatRequestId("create-projection".into()),
            receipt_id: ReceiptId("receipt-create-projection".into()),
            workspace_path: ".".into(),
            selection: Some(AgentChatSelection {
                provider: AgentChatProvider::Claude,
                model: "sonnet".into(),
                effort: AgentChatEffort::Medium,
                mode: AgentChatMode::Agent,
            }),
        })
        .unwrap();
    let [
        AgentChatIntentFrame::Created {
            conversation_id,
            run_id,
            ..
        },
    ] = created.as_slice()
    else {
        panic!("conversation creation must return durable identities");
    };
    (conversation_id.clone(), run_id.clone())
}

fn import(
    runtime: &RuntimeFacade,
    conversation_id: &AgentChatConversationId,
    run_id: &AgentChatRunId,
    range: std::ops::Range<usize>,
) {
    runtime
        .agent_chat_intent(AgentChatIntentFrame::ImportTranscript {
            request_id: AgentChatRequestId(format!("import-{}", range.start)),
            conversation_id: conversation_id.clone(),
            run_id: run_id.clone(),
            entries: range
                .map(|index| HistoricalTranscriptEntry {
                    source_id: format!("entry-{index}"),
                    kind: NormalizedTranscriptKind::AssistantMessage,
                    text: format!("entry {index}"),
                })
                .collect(),
        })
        .unwrap();
}

async fn connect(runtime: &RuntimeFacade) -> DuplexStream {
    let (mut client, server) = duplex(1024 * 1024);
    tokio::spawn(crate::transport::serve_connection(server, runtime.clone()));
    write_frame(
        &mut client,
        &WireFrame::Hello(Hello {
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: CapabilitySet(vec![
                AGENT_CHAT_INTENTS_CAPABILITY.into(),
                AGENT_CHAT_PROJECTION_CAPABILITY.into(),
            ]),
        }),
    )
    .await
    .unwrap();
    assert!(matches!(
        read_frame(&mut client).await.unwrap(),
        WireFrame::Negotiated(_)
    ));
    client
}

async fn follow(
    runtime: &RuntimeFacade,
    conversation_id: &AgentChatConversationId,
    after_cursor: ProjectionCursor,
) -> DuplexStream {
    let mut client = connect(runtime).await;
    write_json_frame(
        &mut client,
        &AgentChatProjectionFrame::FollowConversation {
            request_id: "follow".into(),
            conversation_id: conversation_id.0.clone(),
            after_cursor,
        },
    )
    .await
    .unwrap();
    client
}

async fn deltas(client: &mut DuplexStream, count: usize, after: u64) -> Vec<(u64, String)> {
    let mut previous = after;
    let mut received = Vec::new();
    while received.len() < count {
        let AgentChatProjectionFrame::Delta {
            delta: AgentChatProjectionDelta::Transcript { cursor, event },
            ..
        } = read_json_frame(client).await.unwrap()
        else {
            continue;
        };
        assert!(
            cursor.value > previous,
            "projection cursors strictly increase"
        );
        previous = cursor.value;
        received.push((cursor.value, event.text));
    }
    received
}
