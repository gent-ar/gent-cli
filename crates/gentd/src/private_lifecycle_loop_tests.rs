use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::{
    PrivateLifecycleCommand, PrivateLifecycleLoop, PrivateLifecycleOutcome, PrivateLifecycleOwner,
    PrivateLifecyclePhase, PrivateLifecycleScheduleError,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Call {
    Wake,
    Shutdown,
    Escalate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FakeError {
    Wake,
}

#[derive(Clone, Default)]
struct FakeOwner {
    calls: Rc<RefCell<Vec<Call>>>,
    fail_wake: Rc<Cell<bool>>,
}

impl PrivateLifecycleOwner for FakeOwner {
    type Wake = u8;
    type Shutdown = u8;
    type Escalation = u8;
    type Error = FakeError;

    fn wake(&mut self) -> Result<Self::Wake, Self::Error> {
        self.calls.borrow_mut().push(Call::Wake);
        if self.fail_wake.replace(false) {
            return Err(FakeError::Wake);
        }
        Ok(1)
    }

    fn request_shutdown(&mut self) -> Result<Self::Shutdown, Self::Error> {
        self.calls.borrow_mut().push(Call::Shutdown);
        Ok(2)
    }

    fn escalate_shutdown(&mut self) -> Result<Self::Escalation, Self::Error> {
        self.calls.borrow_mut().push(Call::Escalate);
        Ok(3)
    }
}

#[test]
fn one_item_mailbox_backpressures_before_calling_the_owner() {
    let owner = FakeOwner::default();
    let calls = owner.calls.clone();
    let mut loop_ = PrivateLifecycleLoop::new(owner);

    loop_.schedule(PrivateLifecycleCommand::Wake).unwrap();
    assert_eq!(
        loop_.schedule(PrivateLifecycleCommand::RequestShutdown),
        Err(PrivateLifecycleScheduleError::Backpressured {
            pending: PrivateLifecycleCommand::Wake,
        })
    );
    assert!(calls.borrow().is_empty());
    assert_eq!(
        loop_.tick().unwrap(),
        Some(PrivateLifecycleOutcome::Wake(1))
    );
    assert_eq!(loop_.tick().unwrap(), None);
    assert_eq!(&*calls.borrow(), &[Call::Wake]);
}

#[test]
fn shutdown_order_requires_a_drain_wake_between_process_signals() {
    let owner = FakeOwner::default();
    let calls = owner.calls.clone();
    let mut loop_ = PrivateLifecycleLoop::new(owner);

    loop_.schedule(PrivateLifecycleCommand::Wake).unwrap();
    loop_.tick().unwrap();
    loop_
        .schedule(PrivateLifecycleCommand::RequestShutdown)
        .unwrap();
    assert_eq!(
        loop_.tick().unwrap(),
        Some(PrivateLifecycleOutcome::Shutdown(2))
    );
    assert_eq!(loop_.phase(), PrivateLifecyclePhase::AwaitingDrainWake);
    assert_eq!(
        loop_.schedule(PrivateLifecycleCommand::EscalateShutdown),
        Err(PrivateLifecycleScheduleError::DrainWakeRequired)
    );
    loop_.schedule(PrivateLifecycleCommand::Wake).unwrap();
    loop_.tick().unwrap();
    loop_
        .schedule(PrivateLifecycleCommand::EscalateShutdown)
        .unwrap();
    assert_eq!(
        loop_.tick().unwrap(),
        Some(PrivateLifecycleOutcome::Escalation(3))
    );
    assert_eq!(
        &*calls.borrow(),
        &[Call::Wake, Call::Shutdown, Call::Wake, Call::Escalate]
    );
}

#[test]
fn failed_wake_does_not_unlock_shutdown_or_run_an_implicit_retry() {
    let owner = FakeOwner::default();
    owner.fail_wake.set(true);
    let calls = owner.calls.clone();
    let mut loop_ = PrivateLifecycleLoop::new(owner);

    loop_.schedule(PrivateLifecycleCommand::Wake).unwrap();
    assert_eq!(loop_.tick(), Err(FakeError::Wake));
    assert_eq!(loop_.phase(), PrivateLifecyclePhase::AwaitingWake);
    assert_eq!(
        loop_.schedule(PrivateLifecycleCommand::RequestShutdown),
        Err(PrivateLifecycleScheduleError::WakeRequired)
    );
    assert_eq!(loop_.tick().unwrap(), None);
    assert_eq!(&*calls.borrow(), &[Call::Wake]);
}
