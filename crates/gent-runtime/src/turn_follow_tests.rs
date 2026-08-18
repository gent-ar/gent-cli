use std::sync::{Arc, Mutex};

use gent_ports::{LedgerError, TurnFollowPage, TurnFollowReader};
use gent_types::{
    DurableTurnPhase, HostEpoch, NormalizedTranscriptEvent, NormalizedTranscriptKind, TurnRecord,
};

use super::{TurnFollowRequest, TurnFollowService};

#[derive(Clone)]
struct Source {
    epoch: HostEpoch,
    page: Arc<Mutex<TurnFollowPage>>,
}

impl TurnFollowReader for Source {
    fn turn_follow_host_epoch(&self) -> Result<HostEpoch, LedgerError> {
        Ok(self.epoch)
    }
    fn turn_follow_page(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: u64,
        _: u16,
    ) -> Result<TurnFollowPage, LedgerError> {
        Ok(self.page.lock().unwrap().clone())
    }
}

#[test]
fn terminal_is_only_exposed_after_the_exact_turn_page_is_exhausted() {
    let source = Source {
        epoch: HostEpoch(3),
        page: Arc::new(Mutex::new(page(DurableTurnPhase::Completed, None))),
    };
    assert!(
        TurnFollowService::read(&source, &request())
            .unwrap()
            .terminal
            .is_some()
    );
    source.page.lock().unwrap().next_after_cursor = Some(1);
    assert!(
        TurnFollowService::read(&source, &request())
            .unwrap()
            .terminal
            .is_none()
    );
}

#[test]
fn cursor_token_can_equal_the_last_event_but_not_an_empty_page() {
    let source = Source {
        epoch: HostEpoch(3),
        page: Arc::new(Mutex::new(page(DurableTurnPhase::Active, Some(1)))),
    };
    assert!(TurnFollowService::read(&source, &request()).is_ok());
    let mut page = source.page.lock().unwrap();
    page.events.clear();
    page.next_after_cursor = Some(0);
    drop(page);
    assert!(TurnFollowService::read(&source, &request()).is_err());
}

#[test]
fn cross_turn_events_and_scope_are_fail_closed() {
    let source = Source {
        epoch: HostEpoch(3),
        page: Arc::new(Mutex::new(page(DurableTurnPhase::Active, None))),
    };
    source.page.lock().unwrap().events[0].turn_id = "other".into();
    assert!(TurnFollowService::read(&source, &request()).is_err());
}

fn request() -> TurnFollowRequest {
    TurnFollowRequest {
        conversation_id: "conversation".into(),
        run_id: "run".into(),
        turn_id: "turn".into(),
        after_cursor: 0,
        expected_host_epoch: HostEpoch(3),
        limit: 100,
    }
}

fn page(phase: DurableTurnPhase, next_after_cursor: Option<u64>) -> TurnFollowPage {
    TurnFollowPage {
        turn: TurnRecord {
            turn_id: "turn".into(),
            conversation_id: "conversation".into(),
            run_id: "run".into(),
            sequence: 1,
            phase,
        },
        events: vec![NormalizedTranscriptEvent {
            cursor: 1,
            event_id: "event".into(),
            turn_id: "turn".into(),
            run_id: "run".into(),
            kind: NormalizedTranscriptKind::AssistantMessage,
            text: "done".into(),
            is_partial: false,
        }],
        next_after_cursor,
    }
}
