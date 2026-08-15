//! The composition edge for one public-provider process.
//!
//! This module is the only driver module that combines immutable locks, bounded buffering,
//! session reduction, and interrupt escalation. It only accepts the public Claude and Codex
//! executable identities; private bridges remain outside this crate.

use std::path::PathBuf;

use gent_types::RunVersionLock;

use crate::buffering::{BufferPolicy, FrameBuffer, OfferResult, ReadDirective};
use crate::interrupt::{
    InterruptEvent, InterruptPolicy, InterruptState, ProcessTreeControl, ProcessTreeError,
    transition,
};
use crate::lock::{LockError, recheck};
use crate::session::{DriverSession, OutputLimits, SessionEffect, SessionInput};

pub use crate::launch_spec::LaunchIntent;

/// An immutable, public executable launch request passed to infrastructure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderLaunch {
    pub provider: String,
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub intent: LaunchIntent,
}

/// A live public provider process. Platform implementations own its tree and standard input.
pub trait ProviderProcess: ProcessTreeControl {
    /// Writes one complete provider-native frame to the process standard input.
    ///
    /// # Errors
    /// Returns an error when the owned process cannot accept the frame.
    fn write_frame(&self, frame: &[u8]) -> Result<(), ProcessTreeError>;
}

/// Process-spawning infrastructure. Production implementations belong at an outer edge.
pub trait ProcessLauncher: Send + Sync {
    /// The opaque live process type owned after a successful launch.
    type Process: ProviderProcess + 'static;

    /// Starts the exact executable named by a previously rechecked immutable lock.
    ///
    /// # Errors
    /// Returns an error when the operating system cannot create the provider process tree.
    fn launch(&self, launch: &ProviderLaunch) -> Result<Self::Process, SupervisorError>;
}

/// Errors returned by the supervised public-provider boundary.
#[derive(Debug, thiserror::Error)]
pub enum SupervisorError {
    #[error(transparent)]
    Lock(#[from] LockError),
    #[error("unsupported public provider: {0}")]
    UnsupportedProvider(String),
    #[error("a provider process is already active")]
    ProcessAlreadyActive,
    #[error("no provider process is active")]
    NoActiveProcess,
    #[error(transparent)]
    ProcessTree(#[from] ProcessTreeError),
    #[error("provider launch failed: {0}")]
    Launch(String),
}

/// Owns exactly one public process, while keeping policy reducers independently testable.
#[derive(Debug)]
pub struct ProviderSupervisor<P: ProviderProcess> {
    lock: RunVersionLock,
    session: DriverSession,
    frames: FrameBuffer,
    interrupt: InterruptState,
    process: Option<P>,
}

impl<P: ProviderProcess> ProviderSupervisor<P> {
    /// Creates a supervisor without inspecting or starting a provider.
    #[must_use]
    pub fn new(lock: RunVersionLock, limits: OutputLimits, policy: BufferPolicy) -> Self {
        Self {
            lock,
            session: DriverSession::new(limits),
            frames: FrameBuffer::new(policy),
            interrupt: InterruptState::Running,
            process: None,
        }
    }

    /// Starts a public provider after rechecking the executable identity immediately beforehand.
    ///
    /// # Errors
    /// Returns `ProviderChanged` without launching when the saved executable identity differs.
    pub fn spawn<L: ProcessLauncher<Process = P>>(
        &mut self,
        launcher: &L,
        arguments: Vec<String>,
    ) -> Result<(), SupervisorError> {
        self.launch(launcher, arguments, LaunchIntent::Start)
    }

    /// Resumes a public provider after rechecking the executable identity immediately beforehand.
    ///
    /// # Errors
    /// Returns `ProviderChanged` without launching when the saved executable identity differs.
    pub fn resume<L: ProcessLauncher<Process = P>>(
        &mut self,
        launcher: &L,
        arguments: Vec<String>,
        session_id: String,
    ) -> Result<(), SupervisorError> {
        self.launch(launcher, arguments, LaunchIntent::Resume { session_id })
    }

    /// Offers one complete provider frame; the caller retains it on backpressure.
    pub fn offer_frame(&mut self, frame: Vec<u8>) -> OfferResult {
        self.frames.offer(frame)
    }

    /// Reduces one buffered frame and returns normalized effects for the persistence edge.
    #[must_use]
    pub fn drain_frame(&mut self) -> (Vec<SessionEffect>, Option<ReadDirective>) {
        let (frame, directive) = self.frames.take();
        let effects = frame.map_or_else(Vec::new, |raw| self.apply(SessionInput::RawFrame(raw)));
        (effects, directive)
    }

    /// Writes one already-encoded provider frame only while this supervisor owns a live process.
    ///
    /// # Errors
    /// Returns an error when no process is owned or its input cannot accept the frame.
    pub fn write_frame(&self, frame: &[u8]) -> Result<(), SupervisorError> {
        self.process
            .as_ref()
            .ok_or(SupervisorError::NoActiveProcess)?
            .write_frame(frame)?;
        Ok(())
    }

    /// Reports process termination to the pure session reducer and prevents further signaling.
    #[must_use]
    pub fn process_exited(&mut self, code: Option<i32>) -> Vec<SessionEffect> {
        self.process = None;
        self.interrupt = transition(
            self.interrupt,
            InterruptEvent::Exited,
            InterruptPolicy {
                interrupt_grace_ms: 0,
                terminate_grace_ms: 0,
            },
        )
        .state;
        self.apply(SessionInput::ProcessExited { code })
    }

    /// Delivers one escalation fact to the process tree, if the reducer prescribes a signal.
    ///
    /// # Errors
    /// Returns an error if no process is active or if tree signaling fails.
    pub fn interrupt(
        &mut self,
        event: InterruptEvent,
        policy: InterruptPolicy,
    ) -> Result<Option<u64>, SupervisorError> {
        let next = transition(self.interrupt, event, policy);
        if let Some(signal) = next.signal {
            self.process
                .as_ref()
                .ok_or(SupervisorError::NoActiveProcess)?
                .signal_tree(signal)?;
        }
        self.interrupt = next.state;
        Ok(next.next_wait_ms)
    }

    /// Returns the current pure session state.
    #[must_use]
    pub const fn session(&self) -> &DriverSession {
        &self.session
    }

    fn launch<L: ProcessLauncher<Process = P>>(
        &mut self,
        launcher: &L,
        arguments: Vec<String>,
        intent: LaunchIntent,
    ) -> Result<(), SupervisorError> {
        if self.process.is_some() {
            return Err(SupervisorError::ProcessAlreadyActive);
        }
        let launch = self.launch_request(arguments, intent)?;
        recheck(&self.lock)?;
        self.process = Some(launcher.launch(&launch)?);
        self.interrupt = InterruptState::Running;
        Ok(())
    }

    fn launch_request(
        &self,
        arguments: Vec<String>,
        intent: LaunchIntent,
    ) -> Result<ProviderLaunch, SupervisorError> {
        if !matches!(self.lock.provider.as_str(), "claude" | "codex") {
            return Err(SupervisorError::UnsupportedProvider(
                self.lock.provider.clone(),
            ));
        }
        Ok(ProviderLaunch {
            provider: self.lock.provider.clone(),
            executable: PathBuf::from(&self.lock.canonical_path),
            arguments,
            intent,
        })
    }

    fn apply(&mut self, input: SessionInput) -> Vec<SessionEffect> {
        let transition = self.session.reduce(input);
        self.session = transition.state;
        transition.effects
    }
}
