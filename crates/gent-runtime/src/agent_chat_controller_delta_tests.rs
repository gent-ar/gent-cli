use gent_types::{
    HostEpoch, HostStatus, NormalizedTranscriptEvent, NormalizedTranscriptKind,
    NormalizedTranscriptPage,
};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

use super::{
    AgentChatControllerDeltaReader, AgentChatControllerDeltaRequest, AgentChatControllerDeltaSource,
};
use crate::RuntimeError;

#[derive(Clone)]
struct Source {
    before: HostEpoch,
    after: HostEpoch,
    page: NormalizedTranscriptPage,
    calls: Arc<AtomicU8>,
}

impl AgentChatControllerDeltaSource for Source {
    fn host_status(&self) -> Result<HostStatus, RuntimeError> {
        let epoch = if self.calls.fetch_add(1, Ordering::Relaxed) == 0 {
            self.before
        } else {
            self.after
        };
        Ok(status(epoch))
    }

    fn transcript(
        &self,
        _: &str,
        _: u64,
        _: u16,
    ) -> Result<NormalizedTranscriptPage, RuntimeError> {
        Ok(self.page.clone())
    }
}

#[test]
fn returns_ordered_events_only_for_the_expected_epoch() {
    let source = Source {
        before: HostEpoch(4),
        after: HostEpoch(4),
        page: page(vec![event(3), event(4)], None),
        calls: Arc::new(AtomicU8::new(0)),
    };
    let delta = AgentChatControllerDeltaReader::read(&source, &request(2, HostEpoch(4))).unwrap();
    assert_eq!(delta.events.len(), 2);
    assert_eq!(delta.host_epoch, HostEpoch(4));
}

#[test]
fn rejects_wrong_epoch_and_non_advancing_events() {
    let source = Source {
        before: HostEpoch(4),
        after: HostEpoch(4),
        page: page(vec![event(2)], None),
        calls: Arc::new(AtomicU8::new(0)),
    };
    assert!(AgentChatControllerDeltaReader::read(&source, &request(2, HostEpoch(5))).is_err());
    assert!(AgentChatControllerDeltaReader::read(&source, &request(2, HostEpoch(4))).is_err());
}

#[test]
fn rejects_an_epoch_transition_during_the_delta_read() {
    let source = Source {
        before: HostEpoch(4),
        after: HostEpoch(5),
        page: page(vec![event(3)], None),
        calls: Arc::new(AtomicU8::new(0)),
    };
    assert!(AgentChatControllerDeltaReader::read(&source, &request(2, HostEpoch(4))).is_err());
}

fn request(after_cursor: u64, epoch: HostEpoch) -> AgentChatControllerDeltaRequest {
    AgentChatControllerDeltaRequest {
        conversation_id: "conversation".into(),
        after_cursor,
        expected_host_epoch: epoch,
        limit: 500,
    }
}

fn status(epoch: HostEpoch) -> HostStatus {
    HostStatus {
        host_epoch: epoch,
        protocol_min: 1,
        protocol_max: 1,
        capabilities: gent_types::CapabilitySet::default(),
    }
}

fn page(
    events: Vec<NormalizedTranscriptEvent>,
    next_after_cursor: Option<u64>,
) -> NormalizedTranscriptPage {
    NormalizedTranscriptPage {
        conversation_id: "conversation".into(),
        events,
        next_after_cursor,
    }
}

fn event(cursor: u64) -> NormalizedTranscriptEvent {
    NormalizedTranscriptEvent {
        cursor,
        event_id: format!("event-{cursor}"),
        turn_id: "turn".into(),
        run_id: "run".into(),
        kind: NormalizedTranscriptKind::AssistantMessage,
        text: "normalized".into(),
        is_partial: false,
    }
}
