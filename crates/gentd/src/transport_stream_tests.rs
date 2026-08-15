use std::sync::{Arc, Mutex};
use std::time::Duration;

use gent_protocol::{
    EVENT_STREAM_CAPABILITY, EventStreamFrame, Hello, PublicRunInterruptRequest, PublicRunResponse,
    PublicRunResumeRequest, PublicRunStartRequest, WireFrame, read_frame, read_json_frame,
    write_frame, write_json_frame,
};
use gent_types::{
    CapabilitySet, Command, ConversationStatus, ConversationTimeline, DecisionCommand,
    DecisionSettlement, DoctorReport, Event, EventResume, EventSnapshot, HostEpoch, HostStatus,
    PROTOCOL_MAX, PROTOCOL_MIN, Receipt, ReceiptId,
};
use tokio::io::duplex;

use crate::api::RuntimeApi;
use crate::transport::serve_connection;

#[derive(Clone, Debug)]
struct StreamRuntime {
    events: Arc<Mutex<Vec<Event>>>,
    snapshot: Option<EventSnapshot>,
}

impl RuntimeApi for StreamRuntime {
    fn capabilities(&self) -> Result<CapabilitySet, String> {
        Ok(CapabilitySet(vec![
            "event-resync".into(),
            EVENT_STREAM_CAPABILITY.into(),
            "events".into(),
        ]))
    }
    fn status(&self) -> Result<HostStatus, String> {
        Err("not used".into())
    }
    fn submit(&self, _: Command) -> Result<Receipt, String> {
        Err("not used".into())
    }
    fn resume_events(&self, cursor: u64) -> Result<EventResume, String> {
        let events = self
            .events
            .lock()
            .map_err(|_| "stream events lock poisoned")?
            .iter()
            .filter(|event| event.cursor > cursor)
            .cloned()
            .collect::<Vec<_>>();
        if let Some(snapshot) = self
            .snapshot
            .clone()
            .filter(|snapshot| cursor < snapshot.cursor)
        {
            Ok(EventResume::Resync { snapshot, events })
        } else {
            Ok(EventResume::Delta { events })
        }
    }
    fn doctor(&self) -> DoctorReport {
        DoctorReport::empty()
    }
    fn dependency_plan(
        &self,
        _: gent_protocol::DependencyPlanRequest,
    ) -> gent_protocol::DependencyPlan {
        unreachable!("not used")
    }
    fn dependency_action(
        &self,
        _: gent_protocol::DependencyActionRequest,
    ) -> Result<gent_protocol::DependencyActionResult, String> {
        unreachable!("not used")
    }
    fn submit_decision(
        &self,
        _: DecisionCommand,
    ) -> Result<gent_protocol::DecisionSubmission, String> {
        Err("not used".into())
    }
    fn apply_decision_evidence(
        &self,
        _: String,
        _: gent_protocol::DecisionEvidence,
    ) -> Result<DecisionSettlement, String> {
        Err("not used".into())
    }
    fn start_public_run(&self, _: PublicRunStartRequest) -> Result<PublicRunResponse, String> {
        Err("not used".into())
    }
    fn resume_public_run(&self, _: PublicRunResumeRequest) -> Result<PublicRunResponse, String> {
        Err("not used".into())
    }
    fn interrupt_public_run(
        &self,
        _: PublicRunInterruptRequest,
    ) -> Result<PublicRunResponse, String> {
        Err("not used".into())
    }
    fn conversation_status(&self, _: &str) -> Result<ConversationStatus, String> {
        Err("not used".into())
    }
    fn conversation_timeline(&self, _: &str) -> Result<ConversationTimeline, String> {
        Err("not used".into())
    }
}

fn event(cursor: u64) -> Event {
    Event {
        cursor,
        event_id: format!("event-{cursor}"),
        receipt_id: ReceiptId(format!("receipt-{cursor}")),
        host_epoch: HostEpoch(1),
        kind: "accepted".into(),
        payload: serde_json::json!({}),
    }
}

fn hello(capabilities: Vec<&str>) -> WireFrame {
    WireFrame::Hello(Hello {
        protocol_min: PROTOCOL_MIN,
        protocol_max: PROTOCOL_MAX,
        capabilities: CapabilitySet(capabilities.into_iter().map(str::to_owned).collect()),
    })
}

#[tokio::test]
async fn negotiated_attach_replays_then_delivers_later_events_in_order() {
    let events = Arc::new(Mutex::new(vec![event(1)]));
    let runtime = StreamRuntime {
        events: Arc::clone(&events),
        snapshot: None,
    };
    let (mut client, server) = duplex(16 * 1024);
    let task = tokio::spawn(serve_connection(server, runtime));
    write_frame(
        &mut client,
        &hello(vec!["events", "event-resync", EVENT_STREAM_CAPABILITY]),
    )
    .await
    .unwrap();
    assert!(matches!(
        read_frame(&mut client).await.unwrap(),
        WireFrame::Negotiated(_)
    ));
    write_json_frame(&mut client, &EventStreamFrame::Attach { after_cursor: 0 })
        .await
        .unwrap();
    assert!(matches!(
        read_json_frame::<_, EventStreamFrame>(&mut client).await.unwrap(),
        EventStreamFrame::Replay { events } if events.iter().map(|event| event.cursor).collect::<Vec<_>>() == [1]
    ));
    write_json_frame(&mut client, &EventStreamFrame::Ack { cursor: 1 })
        .await
        .unwrap();
    events.lock().unwrap().push(event(2));
    assert!(matches!(
        tokio::time::timeout(
            Duration::from_millis(250),
            read_json_frame::<_, EventStreamFrame>(&mut client)
        ).await.unwrap().unwrap(),
        EventStreamFrame::Events { events } if events.iter().map(|event| event.cursor).collect::<Vec<_>>() == [2]
    ));
    drop(client);
    assert!(task.await.unwrap().is_ok());
}

#[tokio::test]
async fn attach_is_rejected_without_the_negotiated_stream_capability() {
    let runtime = StreamRuntime {
        events: Arc::new(Mutex::new(Vec::new())),
        snapshot: None,
    };
    let (mut client, server) = duplex(16 * 1024);
    let task = tokio::spawn(serve_connection(server, runtime));
    write_frame(&mut client, &hello(vec!["events"]))
        .await
        .unwrap();
    let _ = read_frame(&mut client).await.unwrap();
    write_json_frame(&mut client, &EventStreamFrame::Attach { after_cursor: 0 })
        .await
        .unwrap();
    assert!(matches!(
        read_frame(&mut client).await.unwrap(),
        WireFrame::Error { code, .. } if code == "invalidCommand"
    ));
    drop(client);
    assert!(task.await.unwrap().is_err());
}

#[tokio::test]
async fn stale_attach_replaces_its_projection_before_receiving_deltas() {
    let runtime = StreamRuntime {
        events: Arc::new(Mutex::new(vec![event(5)])),
        snapshot: Some(EventSnapshot {
            cursor: 4,
            host_epoch: HostEpoch(1),
            schema_version: 1,
            payload: serde_json::json!({ "safe": true }),
        }),
    };
    let (mut client, server) = duplex(16 * 1024);
    let task = tokio::spawn(serve_connection(server, runtime));
    write_frame(
        &mut client,
        &hello(vec!["events", "event-resync", EVENT_STREAM_CAPABILITY]),
    )
    .await
    .unwrap();
    let _ = read_frame(&mut client).await.unwrap();
    write_json_frame(&mut client, &EventStreamFrame::Attach { after_cursor: 0 })
        .await
        .unwrap();
    assert!(matches!(
        read_json_frame::<_, EventStreamFrame>(&mut client).await.unwrap(),
        EventStreamFrame::Resync { snapshot } if snapshot.cursor == 4
    ));
    assert!(matches!(
        read_json_frame::<_, EventStreamFrame>(&mut client).await.unwrap(),
        EventStreamFrame::Events { events } if events.iter().map(|event| event.cursor).collect::<Vec<_>>() == [5]
    ));
    drop(client);
    assert!(task.await.unwrap().is_ok());
}
