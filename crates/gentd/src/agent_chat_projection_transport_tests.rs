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

type FollowScript = Vec<Result<Vec<AgentChatProjectionDelta>, String>>;

#[derive(Clone)]
struct ScriptedFollow(
    std::sync::Arc<std::sync::Mutex<FollowScript>>,
    std::sync::Arc<tokio::sync::watch::Sender<u64>>,
    std::sync::Arc<tokio::sync::watch::Sender<u64>>,
);

impl super::ProjectionFollowPort for ScriptedFollow {
    fn commits(&self) -> tokio::sync::watch::Receiver<u64> {
        self.1.subscribe()
    }

    fn command_catalogs(&self) -> tokio::sync::watch::Receiver<u64> {
        self.2.subscribe()
    }

    fn projection_deltas(
        &self,
        _: &str,
        _: &ProjectionCursor,
    ) -> Result<Vec<AgentChatProjectionDelta>, String> {
        let mut script = self.0.lock().unwrap();
        if script.is_empty() {
            Ok(Vec::new())
        } else {
            script.remove(0)
        }
    }
}

fn scripted(after: u64, script: FollowScript) -> DuplexStream {
    scripted_with_catalogs(after, script, tokio::sync::watch::channel(0).0)
}

fn scripted_with_catalogs(
    after: u64,
    script: FollowScript,
    catalogs: tokio::sync::watch::Sender<u64>,
) -> DuplexStream {
    let (client, server) = duplex(64 * 1024);
    tokio::spawn(super::serve(
        server,
        ScriptedFollow(
            std::sync::Arc::new(std::sync::Mutex::new(script)),
            std::sync::Arc::new(tokio::sync::watch::channel(0).0),
            std::sync::Arc::new(catalogs),
        ),
        "follow".into(),
        "conversation".into(),
        ProjectionCursor { value: after },
    ));
    client
}

#[tokio::test]
async fn a_settled_command_listing_reaches_every_follower_as_a_commands_catalog_delta() {
    let catalogs = tokio::sync::watch::channel(0).0;
    let mut client = scripted_with_catalogs(7, vec![], catalogs.clone());
    tokio::task::yield_now().await;
    catalogs.send_modify(|revision| *revision += 1);
    let frame = tokio::time::timeout(
        Duration::from_secs(5),
        read_json_frame::<_, AgentChatProjectionFrame>(&mut client),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        frame,
        AgentChatProjectionFrame::Delta {
            request_id: "follow".into(),
            delta: AgentChatProjectionDelta::Catalog {
                cursor: ProjectionCursor { value: 7 },
                catalog: gent_protocol::ProjectionCatalog {
                    catalog_id: gent_protocol::COMMAND_CATALOG_ID.into(),
                    selected_id: None,
                    records: vec![],
                },
            },
        }
    );
}

#[tokio::test]
async fn a_failed_follow_read_ends_with_a_typed_error_frame_instead_of_a_dropped_pipe() {
    let mut client = scripted(0, vec![Err("store read failed".into())]);
    let WireFrame::Error { code, message } = read_frame(&mut client).await.unwrap() else {
        panic!("a failed follow must answer with a typed error frame");
    };
    assert_eq!(code, "projectionFollowFailed");
    assert_eq!(message, "store read failed");
}

#[tokio::test]
async fn an_unreadable_record_is_delivered_and_the_follow_keeps_streaming() {
    let unreadable = AgentChatProjectionDelta::UnreadableRecord {
        cursor: ProjectionCursor { value: 8 },
        record_kind: gent_protocol::ProjectionRecordKind::Lifecycle,
    };
    let later = AgentChatProjectionDelta::UnreadableRecord {
        cursor: ProjectionCursor { value: 9 },
        record_kind: gent_protocol::ProjectionRecordKind::Activity,
    };
    let mut client = scripted(
        7,
        vec![Ok(vec![unreadable.clone()]), Ok(vec![later.clone()])],
    );
    let mut received = Vec::new();
    for _ in 0..2 {
        let AgentChatProjectionFrame::Delta { delta, .. } =
            read_json_frame(&mut client).await.unwrap()
        else {
            panic!("an unreadable record must arrive as a projection delta");
        };
        received.push(delta);
    }
    assert_eq!(received, [unreadable, later]);
}
