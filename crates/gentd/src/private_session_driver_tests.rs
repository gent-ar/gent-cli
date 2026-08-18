use std::{cell::RefCell, rc::Rc};

use gent_runtime::ProviderLifecycleEffect;
use gent_types::{HostEpoch, NormalizedProviderEvent};

use super::{
    PrivateSessionDelta, PrivateSessionDrive, PrivateSessionDriver, PrivateSessionEnqueue,
    PrivateSessionEnqueueError, PrivateSessionIngress, PrivateSessionPersisted, PrivateSessionPort,
    PrivateSessionResume, PrivateSessionSnapshot,
};

#[derive(Default)]
struct State {
    cursor: u64,
    terminal: bool,
    calls: Vec<String>,
}

#[derive(Clone)]
struct Store(Rc<RefCell<State>>);

impl PrivateSessionPort for Store {
    type Delta = String;
    type Snapshot = String;
    type Error = ();

    fn persist(
        &mut self,
        ingress: &PrivateSessionIngress,
    ) -> Result<PrivateSessionPersisted<Self::Delta>, Self::Error> {
        let mut state = self.0.borrow_mut();
        state.cursor += 1;
        state.terminal = ingress.is_terminal();
        state.calls.push(ingress.source_id().into());
        Ok(PrivateSessionPersisted {
            cursor: state.cursor,
            delta: ingress.source_id().into(),
            terminal: state.terminal,
        })
    }

    fn snapshot(&self) -> Result<PrivateSessionSnapshot<Self::Snapshot>, Self::Error> {
        let state = self.0.borrow();
        Ok(PrivateSessionSnapshot {
            host_epoch: HostEpoch(7),
            cursor: state.cursor,
            terminal: state.terminal,
            value: format!("snapshot-{}", state.cursor),
        })
    }
}

fn opened() -> (PrivateSessionDriver<Store>, Rc<RefCell<State>>) {
    let state = Rc::new(RefCell::new(State::default()));
    let driver = PrivateSessionDriver::open(Store(state.clone()), HostEpoch(7)).unwrap();
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
        matches!(driver.resume(0).unwrap(), PrivateSessionResume::Delta { deltas, .. } if deltas.is_empty())
    );
    assert_eq!(
        driver.drive().unwrap(),
        PrivateSessionDrive::Persisted {
            cursor: 1,
            terminal: false
        }
    );
    assert_eq!(state.borrow().calls, ["fact-1"]);
    assert!(matches!(
        driver.resume(0).unwrap(),
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
fn stale_or_future_resume_replaces_state_from_a_durable_snapshot() {
    let (mut driver, _) = opened();
    for index in 0..33 {
        driver.enqueue(fact(&format!("fact-{index}"))).unwrap();
        driver.drive().unwrap();
    }
    assert!(
        matches!(driver.resume(0).unwrap(), PrivateSessionResume::Resync(snapshot) if snapshot.cursor == 33)
    );
    assert!(
        matches!(driver.resume(34).unwrap(), PrivateSessionResume::Resync(snapshot) if snapshot.cursor == 33)
    );
}
