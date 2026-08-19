use std::sync::{Arc, Mutex};

use gent_runtime::AgentChatReadService;
use gent_types::AgentChatProvider;

use super::{OrdinaryLifecycleHost, OrdinaryLifecycleRouterError, OrdinaryPublicLifecycleRouter};

struct Host {
    provider: AgentChatProvider,
    events: Arc<Mutex<Vec<&'static str>>>,
    state: HostState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostState {
    Fresh,
    RecoveryArmed,
    RecoveredIdle,
    ShutdownArmed,
    Stopped,
}

impl OrdinaryLifecycleHost for Host {
    fn provider(&self) -> AgentChatProvider {
        self.provider
    }

    fn arm_authority_recovery(&mut self) -> Result<(), ()> {
        self.state = HostState::RecoveryArmed;
        self.events.lock().unwrap().push("recovery");
        Ok(())
    }

    fn wake(&mut self) -> Result<(), ()> {
        self.events.lock().unwrap().push("wake");
        Ok(())
    }

    fn drive(&mut self) -> Result<(), ()> {
        match self.state {
            HostState::RecoveryArmed => self.state = HostState::RecoveredIdle,
            HostState::ShutdownArmed => self.state = HostState::Stopped,
            HostState::Fresh | HostState::RecoveredIdle | HostState::Stopped => {}
        }
        self.events.lock().unwrap().push("drive");
        Ok(())
    }

    fn needs_drive(&self) -> bool {
        matches!(
            self.state,
            HostState::RecoveryArmed | HostState::ShutdownArmed
        )
    }

    fn begin_shutdown_after_recovery(&mut self) -> Result<(), ()> {
        if self.state != HostState::RecoveredIdle {
            return Err(());
        }
        self.state = HostState::ShutdownArmed;
        self.events.lock().unwrap().push("shutdown");
        Ok(())
    }

    fn shutdown_complete(&self) -> bool {
        self.state == HostState::Stopped
    }
}

#[test]
fn shutdown_rejects_unrecovered_hosts_without_fabricating_a_wake() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut router = router(Arc::clone(&events));

    assert_eq!(
        router.begin_shutdown_after_recovery(),
        Err(OrdinaryLifecycleRouterError::HostUnavailable(
            AgentChatProvider::Codex
        ))
    );
    assert!(events.lock().unwrap().is_empty());
    assert!(!router.shutdown_complete());
}

#[test]
fn recovered_idle_hosts_shutdown_without_a_prompt_wake() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut router = router(Arc::clone(&events));

    router.activate_recovery().unwrap();
    router.drive_once().unwrap();
    router.begin_shutdown_after_recovery().unwrap();
    assert_eq!(&*events.lock().unwrap(), &["recovery", "drive", "shutdown"]);
    assert!(!router.shutdown_complete());

    router.drive_once().unwrap();
    assert!(router.shutdown_complete());
    assert_eq!(
        &*events.lock().unwrap(),
        &["recovery", "drive", "shutdown", "drive"]
    );
}

fn router(events: Arc<Mutex<Vec<&'static str>>>) -> OrdinaryPublicLifecycleRouter<()> {
    OrdinaryPublicLifecycleRouter::new(
        AgentChatReadService::new(()),
        vec![Box::new(Host {
            provider: AgentChatProvider::Codex,
            events,
            state: HostState::Fresh,
        })],
    )
    .unwrap()
}
