//! A host stdio owner: bounded framing, one writer, correlated replies and a
//! Windows Job whose wait includes descendants. Never logs child frames/env.
use super::mcp::{Error, MAX_MESSAGE, Peer, Ticket};
use process_wrap::std::ChildWrapper;
#[cfg(not(target_os = "linux"))]
use process_wrap::std::CommandWrap;
use serde_json::Value;
#[cfg(not(target_os = "linux"))]
use std::process::{Command, Stdio};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use zeroize::Zeroize;

pub struct Spec {
    pub launch_guard: Arc<dyn Fn() -> bool + Send + Sync>,
    pub authority: Arc<tempfile::TempDir>,
    pub startup: Option<Ticket>,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: PathBuf,
    pub pins: Vec<File>,
    pub secrets: Vec<String>,
    pub timeout: Duration,
}
impl Drop for Spec {
    fn drop(&mut self) {
        self.args.zeroize();
        for value in self.env.values_mut() {
            value.zeroize();
        }
        self.secrets.zeroize();
    }
}
enum Action {
    Request(
        Value,
        mpsc::SyncSender<Result<Value, Error>>,
        Arc<dyn Fn() -> bool + Send + Sync>,
    ),
    Notify(Value, Arc<dyn Fn() -> bool + Send + Sync>, Option<Ticket>),
}
struct Outbound {
    bytes: Vec<u8>,
    current: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    operation: Option<Ticket>,
}
enum Input {
    Message(Value),
    Closed,
}
struct OwnedChild {
    child: Box<dyn ChildWrapper>,
    settled: bool,
}
impl std::ops::Deref for OwnedChild {
    type Target = dyn ChildWrapper;
    fn deref(&self) -> &Self::Target {
        self.child.as_ref()
    }
}
impl std::ops::DerefMut for OwnedChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.child.as_mut()
    }
}
impl OwnedChild {
    fn settle(&mut self) {
        let _ = self.child.start_kill();
        if self.child.wait().is_err() {
            loop {
                thread::sleep(Duration::from_secs(60));
            }
        }
        self.settled = true;
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !self.settled {
            self.settle();
        }
    }
}
pub struct StdioPeer {
    sender: mpsc::SyncSender<Action>,
    stopped: Arc<AtomicBool>,
    settled: Arc<AtomicBool>,
    timeout: Duration,
    retained: Arc<Mutex<Vec<Ticket>>>,
}
impl StdioPeer {
    /// The caller must supply the host-approved snapshot and retain every
    /// executable/script authority pin in Spec. This is not worker input.
    #[cfg(not(target_os = "linux"))]
    pub fn spawn(spec: Spec) -> Result<Arc<Self>, Error> {
        #[cfg(target_os = "linux")]
        {
            Self::spawn_inner(spec, None)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Self::spawn_inner(spec)
        }
    }
    #[cfg(target_os = "linux")]
    pub fn spawn_owned_linux(
        spec: Spec,
        config: gent_linux_sandbox::host_stdio::Config,
    ) -> Result<Arc<Self>, Error> {
        Self::spawn_inner(spec, Some(config))
    }
    fn spawn_inner(
        spec: Spec,
        #[cfg(target_os = "linux")] native: Option<gent_linux_sandbox::host_stdio::Config>,
    ) -> Result<Arc<Self>, Error> {
        #[cfg(target_os = "linux")]
        crate::provider_egress::mcp_linux_child::validate(
            &spec,
            native.as_ref().ok_or(Error::Unprovable)?,
        )?;

        if !spec.program.is_absolute()
            || !spec.cwd.is_absolute()
            || spec.pins.is_empty()
            || spec.timeout.is_zero()
            || spec.timeout > Duration::from_secs(300)
            || !spec.authority.path().is_dir()
        {
            return Err(Error::Protocol);
        }
        let (sender, receiver) = mpsc::sync_channel(16);
        let (ready, initialized) = mpsc::sync_channel(1);
        let stopped = Arc::new(AtomicBool::new(false));
        let settled = Arc::new(AtomicBool::new(false));
        let retained = Arc::new(Mutex::new(Vec::new()));
        let timeout = spec.timeout;
        let stop = stopped.clone();
        let done = settled.clone();
        let held = retained.clone();
        thread::Builder::new()
            .name("gent-host-mcp-owner".into())
            .spawn(move || {
                #[cfg(target_os = "linux")]
                let result = actor(
                    spec,
                    receiver,
                    ready,
                    owner_actor::Shared {
                        stopped: stop,
                        settled: done,
                        retained: held,
                    },
                    native,
                );
                #[cfg(not(target_os = "linux"))]
                let result = actor(
                    spec,
                    receiver,
                    ready,
                    owner_actor::Shared {
                        stopped: stop,
                        settled: done,
                        retained: held,
                    },
                );
                if result.is_err() { /* static failure only */ }
            })
            .map_err(|_| Error::Unprovable)?;
        initialized
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| {
                stopped.store(true, Ordering::Release);
                Error::Unprovable
            })??;
        Ok(Arc::new(Self {
            sender,
            stopped,
            settled,
            timeout,
            retained,
        }))
    }
    #[cfg(all(test, windows))]
    pub fn wait_settled(&self, duration: Duration) -> bool {
        let end = Instant::now() + duration;
        while !self.settled.load(Ordering::Acquire) && Instant::now() < end {
            thread::sleep(Duration::from_millis(10));
        }
        self.settled.load(Ordering::Acquire)
    }
}
impl Peer for StdioPeer {
    fn retain_until_settled(&self, ticket: Ticket) {
        let mut held = self
            .retained
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.settled.load(Ordering::Acquire) {
            held.push(ticket);
        }
    }
    fn request(&self, value: &Value) -> Result<Value, Error> {
        self.request_bound(value, Arc::new(|| true))
    }
    fn request_bound(
        &self,
        value: &Value,
        current: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Result<Value, Error> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(Error::Unprovable);
        }
        let (sender, response) = mpsc::sync_channel(1);
        self.sender
            .try_send(Action::Request(value.clone(), sender, current))
            .map_err(|_| Error::Busy)?;
        response
            .recv_timeout(self.timeout + Duration::from_secs(1))
            .map_err(|_| {
                self.stopped.store(true, Ordering::Release);
                Error::Unprovable
            })?
    }
    fn notify(&self, value: &Value) -> Result<(), Error> {
        self.notify_bound(value, Arc::new(|| true), None)
    }
    fn notify_bound(
        &self,
        value: &Value,
        current: Arc<dyn Fn() -> bool + Send + Sync>,
        ticket: Option<Ticket>,
    ) -> Result<(), Error> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(Error::Unprovable);
        }
        self.sender
            .try_send(Action::Notify(value.clone(), current, ticket))
            .map_err(|_| Error::Busy)
    }
    fn revoke(&self) {
        self.stopped.store(true, Ordering::Release);
    }
    fn settled(&self) -> bool {
        self.settled.load(Ordering::Acquire)
    }
}
impl Drop for StdioPeer {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
    }
}

#[path = "provider_mcp_stdio_actor.rs"]
mod owner_actor;
use owner_actor::actor;
