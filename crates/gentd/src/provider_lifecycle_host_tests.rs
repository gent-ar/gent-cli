use std::cell::RefCell;
use std::rc::Rc;

use super::{
    ProviderLifecycleHost, ProviderLifecycleHostError, ProviderLifecycleWake,
    ProviderLifecycleWakePort,
};
use crate::private_lifecycle_loop::{
    PrivateLifecycleCommand, PrivateLifecycleOutcome, PrivateLifecycleOwner,
    PrivateLifecycleScheduleError,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Call {
    Wake,
    Shutdown,
    Escalate,
}

#[derive(Clone, Default)]
struct Owner(Rc<RefCell<Vec<Call>>>);

impl PrivateLifecycleOwner for Owner {
    type Wake = u8;
    type Shutdown = u8;
    type Escalation = u8;
    type Error = ();

    fn wake(&mut self) -> Result<Self::Wake, Self::Error> {
        self.0.borrow_mut().push(Call::Wake);
        Ok(1)
    }

    fn request_shutdown(&mut self) -> Result<Self::Shutdown, Self::Error> {
        self.0.borrow_mut().push(Call::Shutdown);
        Ok(2)
    }

    fn escalate_shutdown(&mut self) -> Result<Self::Escalation, Self::Error> {
        self.0.borrow_mut().push(Call::Escalate);
        Ok(3)
    }
}

#[test]
fn unarmed_host_has_no_owner_call_or_control_route() {
    let owner = Owner::default();
    let calls = owner.0.clone();
    let mut host = ProviderLifecycleHost::new(owner);

    assert!(!host.is_armed());
    assert_eq!(host.drive().unwrap(), None);
    assert_eq!(
        host.request_shutdown(),
        Err(ProviderLifecycleHostError::Inactive)
    );
    assert!(calls.borrow().is_empty());
}

#[test]
fn chat_commit_port_only_arms_the_private_host() {
    let owner = Owner::default();
    let calls = owner.0.clone();
    let mut host = ProviderLifecycleHost::new(owner);

    host.wake_after_prompt_commit().unwrap();
    assert!(host.is_armed());
    assert!(calls.borrow().is_empty());
}

#[test]
fn committed_prompt_arms_one_bounded_wake_and_coalesces_the_next() {
    let owner = Owner::default();
    let calls = owner.0.clone();
    let mut host = ProviderLifecycleHost::new(owner);

    assert_eq!(
        host.wake_after_prompt_commit().unwrap(),
        ProviderLifecycleWake::Armed
    );
    assert_eq!(
        host.wake_after_prompt_commit().unwrap(),
        ProviderLifecycleWake::Coalesced
    );
    assert!(calls.borrow().is_empty());
    assert_eq!(
        host.drive().unwrap(),
        Some(PrivateLifecycleOutcome::Wake(1))
    );
    assert_eq!(&*calls.borrow(), &[Call::Wake]);
    assert_eq!(
        host.drive().unwrap(),
        Some(PrivateLifecycleOutcome::Wake(1))
    );
    assert_eq!(&*calls.borrow(), &[Call::Wake, Call::Wake]);
}

#[test]
fn pending_wake_backpressures_shutdown_until_one_drive_completes() {
    let mut host = ProviderLifecycleHost::new(Owner::default());
    host.wake_after_prompt_commit().unwrap();

    assert_eq!(
        host.request_shutdown(),
        Err(ProviderLifecycleHostError::Schedule(
            PrivateLifecycleScheduleError::Backpressured {
                pending: PrivateLifecycleCommand::Wake,
            }
        ))
    );
    host.drive().unwrap();
    host.request_shutdown().unwrap();
}

#[test]
fn drain_wake_is_required_before_an_escalation() {
    let owner = Owner::default();
    let calls = owner.0.clone();
    let mut host = ProviderLifecycleHost::new(owner);
    host.wake_after_prompt_commit().unwrap();
    host.drive().unwrap();
    host.request_shutdown().unwrap();
    assert_eq!(
        host.drive().unwrap(),
        Some(PrivateLifecycleOutcome::Shutdown(2))
    );
    assert!(matches!(
        host.escalate_shutdown(),
        Err(ProviderLifecycleHostError::Schedule(
            PrivateLifecycleScheduleError::DrainWakeRequired
        ))
    ));
    assert_eq!(
        host.drive().unwrap(),
        Some(PrivateLifecycleOutcome::Wake(1))
    );
    host.escalate_shutdown().unwrap();
    assert_eq!(
        host.drive().unwrap(),
        Some(PrivateLifecycleOutcome::Escalation(3))
    );
    assert_eq!(
        &*calls.borrow(),
        &[Call::Wake, Call::Shutdown, Call::Wake, Call::Escalate]
    );
}
