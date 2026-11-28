use std::sync::{Arc, Mutex};
use std::time::Duration;

use gent_protocol::{AgentChatIntentFrame, AgentChatTranscriptFrame, read_json_frame};
use gent_types::{
    AgentChatConversationId, AgentChatRequestId, NormalizedTranscriptEvent,
    NormalizedTranscriptKind, NormalizedTranscriptPage,
};
use tokio::io::{DuplexStream, duplex};
use tokio::sync::watch;

use super::{send_events, serve};
use crate::agent_chat_subscription::TranscriptPort;

#[derive(Clone)]
struct TranscriptRuntime {
    events: Arc<Mutex<Vec<NormalizedTranscriptEvent>>>,
    landing: Arc<Mutex<Option<NormalizedTranscriptEvent>>>,
    commits: Arc<watch::Sender<u64>>,
    reads: Arc<Mutex<usize>>,
}

impl TranscriptRuntime {
    fn new(events: Vec<NormalizedTranscriptEvent>) -> Self {
        Self {
            events: Arc::new(Mutex::new(events)),
            landing: Arc::new(Mutex::new(None)),
            commits: Arc::new(watch::channel(0).0),
            reads: Arc::new(Mutex::new(0)),
        }
    }

    fn commit(&self, event: NormalizedTranscriptEvent) {
        self.events.lock().unwrap().push(event);
        self.commits.send_modify(|commits| *commits += 1);
    }

    fn reads(&self) -> usize {
        *self.reads.lock().unwrap()
    }
}

impl TranscriptPort for TranscriptRuntime {
    fn commits(&self) -> watch::Receiver<u64> {
        self.commits.subscribe()
    }

    fn transcript(
        &self,
        frame: AgentChatTranscriptFrame,
    ) -> Result<AgentChatTranscriptFrame, String> {
        let AgentChatTranscriptFrame::PageRequest {
            conversation_id,
            after_cursor,
            ..
        } = frame
        else {
            return Err("request expected".into());
        };
        *self.reads.lock().unwrap() += 1;
        let events = self
            .events
            .lock()
            .map_err(|_| "transcript lock poisoned")?
            .iter()
            .filter(|event| event.cursor > after_cursor.unwrap_or_default())
            .cloned()
            .collect();
        if let Some(late) = self.landing.lock().unwrap().take() {
            self.commit(late);
        }
        Ok(AgentChatTranscriptFrame::Page(NormalizedTranscriptPage {
            conversation_id,
            events,
            next_after_cursor: None,
        }))
    }
}

fn subscribe(runtime: TranscriptRuntime) -> (DuplexStream, tokio::task::JoinHandle<()>) {
    let (client, server) = duplex(32 * 1024);
    let task = tokio::spawn(async move {
        serve(
            server,
            runtime,
            AgentChatRequestId("request-1".into()),
            AgentChatConversationId("conversation-1".into()),
            0,
        )
        .await
        .unwrap();
    });
    (client, task)
}

async fn next_cursor(client: &mut DuplexStream) -> u64 {
    match tokio::time::timeout(
        Duration::from_secs(1),
        read_json_frame::<_, AgentChatIntentFrame>(client),
    )
    .await
    .expect("a committed event reaches the subscriber")
    .unwrap()
    {
        AgentChatIntentFrame::SubscriptionEvent { event, .. } => event.cursor,
        frame => panic!("unexpected frame {frame:?}"),
    }
}

#[tokio::test]
async fn subscription_replays_then_waits_for_later_durable_events() {
    let runtime = TranscriptRuntime::new(vec![event(1)]);
    let (mut client, task) = subscribe(runtime.clone());
    assert_eq!(next_cursor(&mut client).await, 1);
    runtime.commit(event(2));
    assert_eq!(next_cursor(&mut client).await, 2);
    drop(client);
    task.await.unwrap();
}

#[tokio::test]
async fn a_write_landing_between_the_read_and_the_wait_is_not_lost() {
    let runtime = TranscriptRuntime::new(Vec::new());
    *runtime.landing.lock().unwrap() = Some(event(1));
    let (mut client, task) = subscribe(runtime);
    assert_eq!(next_cursor(&mut client).await, 1);
    drop(client);
    task.await.unwrap();
}

#[tokio::test]
async fn idle_subscriptions_read_the_ledger_only_when_a_commit_lands() {
    let runtime = TranscriptRuntime::new(vec![event(1)]);
    let mut followers = Vec::new();
    for _ in 0..10 {
        let (mut client, task) = subscribe(runtime.clone());
        assert_eq!(next_cursor(&mut client).await, 1);
        followers.push((client, task));
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(runtime.reads(), 20);
    runtime.commit(event(2));
    for (client, _) in &mut followers {
        assert_eq!(next_cursor(client).await, 2);
    }
    for (client, task) in followers {
        drop(client);
        task.await.unwrap();
    }
}

#[tokio::test]
async fn oversized_events_are_rejected_before_writing() {
    let (mut reader, mut writer) = duplex(32 * 1024);
    let page = NormalizedTranscriptPage {
        conversation_id: "conversation-1".into(),
        events: vec![NormalizedTranscriptEvent {
            text: "x".repeat(gent_protocol::MAX_FRAME_BYTES),
            ..event(1)
        }],
        next_after_cursor: None,
    };
    assert!(
        send_events(&mut writer, &AgentChatRequestId("request".into()), 0, page)
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(
            Duration::from_millis(20),
            read_json_frame::<_, AgentChatIntentFrame>(&mut reader)
        )
        .await
        .is_err()
    );
}

fn event(cursor: u64) -> NormalizedTranscriptEvent {
    NormalizedTranscriptEvent {
        cursor,
        event_id: format!("event-{cursor}"),
        turn_id: "turn-1".into(),
        run_id: "run-1".into(),
        kind: NormalizedTranscriptKind::AssistantMessage,
        text: "normalized".into(),
        is_partial: false,
        failure_code: None,
        resets_at: None,
        recorded_at_unix_ms: None,
        origin: None,
        attachments: Vec::new(),
    }
}
