//! No host process starts until the native worker's workspace/proof was verified.
use super::{
    mcp::{Error, Peer, Ticket},
    mcp_stdio::{Spec, StdioPeer},
};
use serde_json::Value;
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
enum State {
    Dormant(Box<Option<Spec>>),
    Starting,
    Ready(Arc<StdioPeer>),
    Unknown,
}
pub struct DeferredPeer {
    state: Mutex<State>,
    changed: Condvar,
    stopped: Arc<AtomicBool>,
    held: Mutex<Vec<Ticket>>,
    #[cfg(target_os = "linux")]
    native: Mutex<Option<gent_linux_sandbox::host_stdio::Config>>,
}
impl DeferredPeer {
    pub fn new(mut spec: Spec) -> Arc<Self> {
        let stopped = Arc::new(AtomicBool::new(false));
        let guard = spec.launch_guard.clone();
        let revoke = stopped.clone();
        spec.launch_guard = Arc::new(move || guard() && !revoke.load(Ordering::Acquire));
        Arc::new(Self {
            state: Mutex::new(State::Dormant(Box::new(Some(spec)))),
            changed: Condvar::new(),
            stopped,
            held: Mutex::new(Vec::new()),
            #[cfg(target_os = "linux")]
            native: Mutex::new(None),
        })
    }
    /// Explicit native ownership; no host-process fallback on missing config.
    #[cfg(target_os = "linux")]
    pub fn new_owned_linux(
        spec: Spec,
        config: gent_linux_sandbox::host_stdio::Config,
    ) -> Result<Arc<Self>, Error> {
        super::mcp_linux_child::validate(&spec, &config)?;
        let peer = Self::new(spec);
        *peer.native.lock().map_err(|_| Error::Unprovable)? = Some(config);
        Ok(peer)
    }
    /// The only production caller invokes this after `EvidenceGate::publish`.
    pub fn activate(&self) -> Result<(), Error> {
        let spec = {
            let mut state = self.state.lock().map_err(|_| Error::Unprovable)?;
            if self.stopped.load(Ordering::Acquire) {
                return Err(Error::Revoked);
            }
            let State::Dormant(spec) = &mut *state else {
                return Err(Error::Protocol);
            };
            let spec = spec.take().ok_or(Error::Protocol)?;
            *state = State::Starting;
            spec
        };
        #[cfg(target_os = "linux")]
        let spawned = self
            .native
            .lock()
            .map_err(|_| Error::Unprovable)?
            .take()
            .ok_or(Error::Unprovable)
            .and_then(|config| StdioPeer::spawn_owned_linux(spec, config));
        #[cfg(not(target_os = "linux"))]
        let spawned = StdioPeer::spawn(spec);
        let mut state = self.state.lock().map_err(|_| Error::Unprovable)?;
        match spawned {
            Ok(peer) => {
                if self.stopped.load(Ordering::Acquire) {
                    peer.revoke();
                }
                for ticket in self.held.lock().map_err(|_| Error::Unprovable)?.drain(..) {
                    peer.retain_until_settled(ticket);
                }
                *state = State::Ready(peer);
                self.changed.notify_all();
                Ok(())
            }
            Err(Error::Revoked) => {
                self.stopped.store(true, Ordering::Release);
                *state = State::Dormant(Box::new(None));
                self.changed.notify_all();
                Err(Error::Revoked)
            }
            Err(_) => {
                *state = State::Unknown;
                self.changed.notify_all();
                Err(Error::Unprovable)
            }
        }
    }
    fn ready(&self) -> Result<Arc<StdioPeer>, Error> {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut state = self.state.lock().map_err(|_| Error::Unprovable)?;
        loop {
            if self.stopped.load(Ordering::Acquire) {
                return Err(Error::Revoked);
            }
            match &*state {
                State::Ready(peer) => return Ok(peer.clone()),
                State::Unknown => return Err(Error::Unprovable),
                _ => (),
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::NotReady);
            }
            state = self
                .changed
                .wait_timeout(state, remaining)
                .map_err(|_| Error::Unprovable)?
                .0;
        }
    }
}
impl Peer for DeferredPeer {
    fn request(&self, message: &Value) -> Result<Value, Error> {
        self.ready()?.request(message)
    }
    fn request_bound(
        &self,
        message: &Value,
        guard: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Result<Value, Error> {
        self.ready()?.request_bound(message, guard)
    }
    fn notify(&self, message: &Value) -> Result<(), Error> {
        self.ready()?.notify(message)
    }
    fn notify_bound(
        &self,
        message: &Value,
        guard: Arc<dyn Fn() -> bool + Send + Sync>,
        ticket: Option<Ticket>,
    ) -> Result<(), Error> {
        self.ready()?.notify_bound(message, guard, ticket)
    }
    fn retain_until_settled(&self, ticket: Ticket) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let State::Ready(peer) = &*state {
            peer.retain_until_settled(ticket);
        } else if !self.stopped.load(Ordering::Acquire) || !matches!(*state, State::Dormant(_)) {
            self.held
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(ticket);
        }
    }
    fn revoke(&self) {
        self.stopped.store(true, Ordering::Release);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &mut *state {
            State::Ready(peer) => peer.revoke(),
            State::Dormant(spec) => {
                spec.take();
                self.held
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clear();
            }
            _ => (),
        }
        self.changed.notify_all();
    }
    fn settled(&self) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &*state {
            State::Ready(peer) => peer.settled(),
            State::Dormant(_) => self.stopped.load(Ordering::Acquire),
            _ => false,
        }
    }
}
impl Drop for DeferredPeer {
    fn drop(&mut self) {
        self.revoke();
        if !self.settled() {
            let held = self
                .held
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::forget(std::mem::take(held));
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    fn fixture() -> (Arc<tempfile::TempDir>, Arc<DeferredPeer>) {
        fixture_guard(Arc::new(|| true))
    }
    fn fixture_guard(
        guard: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> (Arc<tempfile::TempDir>, Arc<DeferredPeer>) {
        let root = Arc::new(tempfile::tempdir().unwrap());
        let script = root.path().join("server.ps1");
        std::fs::write(&script, "Set-Content spawned.txt started; while ($null -ne ($line = [Console]::In.ReadLine())) { $m = ConvertFrom-Json $line; [Console]::Out.WriteLine((ConvertTo-Json -Compress @{jsonrpc='2.0'; id=$m.id; result=@{tools=@()}})); [Console]::Out.Flush() }").unwrap();
        let powershell = super::super::mcp_stdio_scope_tests::powershell();
        let peer = DeferredPeer::new(Spec {
            launch_guard: guard,
            authority: root.clone(),
            startup: None,
            program: powershell.clone(),
            args: super::super::mcp_stdio_scope_tests::script_arguments(&script),
            env: BTreeMap::new(),
            cwd: root.path().into(),
            pins: vec![
                std::fs::File::open(powershell).unwrap(),
                std::fs::File::open(script).unwrap(),
            ],
            secrets: vec![],
            timeout: Duration::from_secs(30),
        });
        (root, peer)
    }
    #[test]
    fn host_mcp_deferred_does_not_spawn_until_explicit_verified_activation() {
        let (root, peer) = fixture();
        let request = peer.clone();
        let call = std::thread::spawn(move || {
            request.request(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
        });
        std::thread::sleep(Duration::from_millis(100));
        assert!(!root.path().join("spawned.txt").exists());
        peer.activate().unwrap();
        assert!(call.join().unwrap().is_ok());
        assert!(root.path().join("spawned.txt").exists());
        peer.revoke();
        let end = Instant::now() + Duration::from_secs(5);
        while !peer.settled() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(peer.settled());
    }
    #[test]
    fn host_mcp_deferred_revocation_before_native_admission_never_starts_host_code() {
        let (root, peer) = fixture();
        peer.revoke();
        assert!(peer.settled());
        assert!(matches!(peer.activate(), Err(Error::Revoked)));
        assert!(!root.path().join("spawned.txt").exists());
    }
    #[test]
    fn host_mcp_deferred_revoke_between_publish_and_spawn_is_checked_inside_actor() {
        let barrier = Arc::new((Mutex::new((false, false)), Condvar::new()));
        let for_actor = barrier.clone();
        let guard = Arc::new(move || {
            let (lock, wake) = &*for_actor;
            let mut state = lock.lock().unwrap();
            state.0 = true;
            wake.notify_all();
            let deadline = Instant::now() + Duration::from_secs(3);
            while !state.1 && Instant::now() < deadline {
                state = wake
                    .wait_timeout(state, Duration::from_millis(20))
                    .unwrap()
                    .0;
            }
            true
        });
        let (root, peer) = fixture_guard(guard);
        let activate = peer.clone();
        let spawned = std::thread::spawn(move || activate.activate());
        let (lock, wake) = &*barrier;
        let mut state = lock.lock().unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !state.0 && Instant::now() < deadline {
            state = wake
                .wait_timeout(state, Duration::from_millis(20))
                .unwrap()
                .0;
        }
        assert!(state.0);
        peer.revoke();
        state.1 = true;
        wake.notify_all();
        drop(state);
        assert!(matches!(spawned.join().unwrap(), Err(Error::Revoked)));
        assert!(peer.settled());
        assert!(!root.path().join("spawned.txt").exists());
    }
}
