use std::{cell::RefCell, rc::Rc};

use gent_runtime::ProviderLifecycleEffect;
use gent_types::NormalizedProviderEvent;

use super::{
    PrivateSessionDelta, PrivateSessionDrive, PrivateSessionDriver, PrivateSessionEnqueue,
    PrivateSessionEnqueueError, PrivateSessionError, PrivateSessionIngress, PrivateSessionResume,
};
use crate::private_session_atomic_port::{
    PrivateSessionAtomicBatch, PrivateSessionAtomicPort, PrivateSessionAtomicRecord,
};

#[derive(Default)]
struct State {
    cursor: u64,
    terminal: bool,
    calls: Vec<String>,
}

#[derive(Clone)]
struct Store(Rc<RefCell<State>>);

impl PrivateSessionAtomicPort for Store {
    type Delta = String;
    type Error = ();

    fn persist_atomic_batch(
        &mut self,
        ingress: &[PrivateSessionIngress],
    ) -> Result<PrivateSessionAtomicBatch<Self::Delta>, Self::Error> {
        let mut state = self.0.borrow_mut();
        let records = ingress
            .iter()
            .map(|fact| {
                state.cursor += 1;
                state.terminal = fact.is_terminal();
                state.calls.push(fact.source_id().into());
                PrivateSessionAtomicRecord {
                    source_id: fact.source_id().into(),
                    cursor: state.cursor,
                    delta: fact.source_id().into(),
                    terminal: state.terminal,
                }
            })
            .collect();
        Ok(PrivateSessionAtomicBatch { records })
    }
}

fn opened() -> (PrivateSessionDriver<Store>, Rc<RefCell<State>>) {
    let state = Rc::new(RefCell::new(State::default()));
    let driver = PrivateSessionDriver::open(Store(state.clone()));
    (driver, state)
}

fn fact(id: &str) -> PrivateSessionIngress {
    PrivateSessionIngress {
        source_id: id.into(),
        effect: ProviderLifecycleEffect::Normalized(NormalizedProviderEvent::TurnStarted {
            turn_id: "turn-1".into(),
        }),
    }
}

#[test]
fn persistence_precedes_visible_delta_and_duplicate_is_not_rebroadcast() {
    let (mut driver, state) = opened();
    assert_eq!(
        driver.enqueue(fact("fact-1")).unwrap(),
        PrivateSessionEnqueue::Queued
    );
    assert!(
        matches!(driver.resume(0), PrivateSessionResume::Delta { deltas, .. } if deltas.is_empty())
    );
    assert_eq!(
        driver.drive().unwrap(),
        PrivateSessionDrive::Persisted {
            cursors: vec![1],
            terminal: false
        }
    );
    assert_eq!(state.borrow().calls, ["fact-1"]);
    assert!(matches!(
        driver.resume(0),
        PrivateSessionResume::Delta { deltas, terminal: false }
            if deltas == vec![PrivateSessionDelta { cursor: 1, value: "fact-1".into() }]
    ));
    assert_eq!(
        driver.enqueue(fact("fact-1")).unwrap(),
        PrivateSessionEnqueue::AlreadyPersisted { cursor: 1 }
    );
    assert_eq!(state.borrow().calls, ["fact-1"]);
}

#[test]
fn bounded_queue_and_terminal_settlement_fence_future_input() {
    let (mut driver, _) = opened();
    for index in 0..16 {
        assert_eq!(
            driver.enqueue(fact(&format!("fact-{index}"))).unwrap(),
            PrivateSessionEnqueue::Queued
        );
    }
    assert_eq!(
        driver.enqueue(fact("overflow")),
        Err(PrivateSessionEnqueueError::Backpressured)
    );
    assert!(matches!(
        driver.drive().unwrap(),
        PrivateSessionDrive::Persisted { .. }
    ));
    assert_eq!(
        driver
            .enqueue(PrivateSessionIngress {
                source_id: "terminal".into(),
                effect: ProviderLifecycleEffect::Terminal {
                    reason: "completed".into()
                }
            })
            .unwrap(),
        PrivateSessionEnqueue::Queued
    );
    while !matches!(
        driver.drive().unwrap(),
        PrivateSessionDrive::Persisted { terminal: true, .. }
    ) {}
    assert_eq!(
        driver.enqueue(fact("late")),
        Err(PrivateSessionEnqueueError::TerminalSettled)
    );
}

#[test]
fn stale_or_future_resume_requires_a_durable_cursor_replay() {
    let (mut driver, _) = opened();
    for index in 0..33 {
        driver.enqueue(fact(&format!("fact-{index}"))).unwrap();
        driver.drive().unwrap();
    }
    assert!(matches!(
        driver.resume(0),
        PrivateSessionResume::ReplayRequired {
            through_cursor: 33,
            ..
        }
    ));
    assert!(matches!(
        driver.resume(34),
        PrivateSessionResume::ReplayRequired {
            through_cursor: 33,
            ..
        }
    ));
}

#[test]
fn committed_batch_exposes_all_ordered_projection_cursors() {
    let (mut driver, state) = opened();
    for id in ["one", "two", "three"] {
        driver.enqueue(fact(id)).unwrap();
    }
    assert_eq!(
        driver.drive().unwrap(),
        PrivateSessionDrive::Persisted {
            cursors: vec![1, 2, 3],
            terminal: false,
        }
    );
    assert_eq!(state.borrow().calls, ["one", "two", "three"]);
    assert!(matches!(
        driver.resume(0),
        PrivateSessionResume::Delta { deltas, .. }
            if deltas.iter().map(|delta| delta.cursor).collect::<Vec<_>>() == [1, 2, 3]
    ));
}

#[derive(Clone)]
struct MismatchedStore;

impl PrivateSessionAtomicPort for MismatchedStore {
    type Delta = String;
    type Error = ();

    fn persist_atomic_batch(
        &mut self,
        _: &[PrivateSessionIngress],
    ) -> Result<PrivateSessionAtomicBatch<Self::Delta>, Self::Error> {
        Ok(PrivateSessionAtomicBatch {
            records: vec![PrivateSessionAtomicRecord {
                source_id: "other".into(),
                cursor: 1,
                delta: "other".into(),
                terminal: false,
            }],
        })
    }
}

#[test]
fn malformed_atomic_batch_is_not_published_or_dequeued() {
    let mut driver = PrivateSessionDriver::open(MismatchedStore);
    driver.enqueue(fact("expected")).unwrap();
    assert!(matches!(
        driver.drive(),
        Err(PrivateSessionError::SourceIdMismatch)
    ));
    assert!(matches!(
        driver.resume(0),
        PrivateSessionResume::Delta { deltas, .. } if deltas.is_empty()
    ));
}
