//! The one public-provider composition edge: locks, bounded buffering, reduction, and interrupts.
//! It accepts only Claude/Codex identities; private bridges remain outside this crate.
use crate::buffering::{BufferPolicy, OfferResult, ReadDirective};
use crate::interrupt::{
    InterruptEvent, InterruptPolicy, InterruptState, ProcessTreeError, transition,
};
pub use crate::launch_spec::LaunchIntent;
use crate::lock::{LockError, recheck};
use crate::output_pump::{MAX_OUTPUT_CHUNK_BYTES, OutputPumpError, ProviderOutputPump};
use crate::session::{DriverSession, OutputLimits, SessionEffect, SessionInput};
use gent_types::{RunVersionLock, SandboxWorkspaceAccess};
use std::path::PathBuf;
/// An immutable, public executable launch request passed to infrastructure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderLaunch {
    pub permission_binding: Option<gent_types::SandboxPermissionBinding>,
    pub run_id: Option<String>,
    pub lock: RunVersionLock,
    pub provider: String,
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub intent: LaunchIntent,
    /// Durable workspace resolved by Gent for this run; never taken from a provider frame.
    pub workspace_root: Option<PathBuf>,
    /// Exact filesystem authority derived from the durable run selection.
    pub workspace_access: SandboxWorkspaceAccess,
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

    /// Whether this launch must fail before protocol I/O without live OS evidence.
    fn requires_sandbox_evidence(&self, _launch: &ProviderLaunch) -> bool {
        false
    }

    fn validate_permission_binding(
        &self,
        binding: &gent_types::SandboxPermissionBinding,
    ) -> Result<(), SupervisorError> {
        if binding.policy().requires_sandbox() {
            return Err(SupervisorError::SandboxUnavailable(
                "verified contained provider composition unavailable".into(),
            ));
        }
        Ok(())
    }
}

/// Errors returned by the supervised public-provider boundary.
#[derive(Debug, thiserror::Error)]
pub enum SupervisorError {
    #[error(transparent)]
    Lock(#[from] LockError),
    #[error("unsupported public provider: {0}")]
    UnsupportedProvider(String),
    #[error("the sandbox is unavailable on this device: {0}")]
    SandboxUnavailable(String),
    #[error("the sandbox refused this launch: {0}")]
    SandboxAdmission(String),
    #[error("a provider process is already active")]
    ProcessAlreadyActive,
    #[error("no provider process is active")]
    NoActiveProcess,
    #[error(transparent)]
    ProcessTree(#[from] ProcessTreeError),
    #[error(transparent)]
    Output(#[from] OutputPumpError),
    #[error("provider launch failed: {0}")]
    Launch(String),
}

/// Owns exactly one public process, while keeping policy reducers independently testable.
#[derive(Debug)]
pub struct ProviderSupervisor<P: ProviderProcess> {
    lock: RunVersionLock,
    session: DriverSession,
    output: ProviderOutputPump,
    interrupt: InterruptState,
    process: Option<P>,
}

impl<P: ProviderProcess> ProviderSupervisor<P> {
    /// Creates a supervisor without inspecting or starting a provider.
    ///
    /// # Panics
    /// Panics only if an internal fixed output-pump limit or a previously validated
    /// [`BufferPolicy`] becomes invalid.
    #[must_use]
    pub fn new(lock: RunVersionLock, limits: OutputLimits, policy: BufferPolicy) -> Self {
        Self {
            lock,
            session: DriverSession::new(limits),
            output: ProviderOutputPump::new(MAX_OUTPUT_CHUNK_BYTES, policy.max_bytes, policy)
                .expect("fixed non-zero output chunk limit and validated buffer policy"),
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
        self.output.offer_frame(frame)
    }

    /// Frames one bounded stdout chunk and directs the process reader to continue or pause.
    ///
    /// # Errors
    /// Returns an error for an oversized, malformed, or prematurely delivered chunk.
    pub fn offer_output_chunk(&mut self, chunk: &[u8]) -> Result<ReadDirective, OutputPumpError> {
        self.output.accept_chunk(chunk)
    }

    /// Pulls one real process stdout chunk through the bounded pump and session reducer.
    ///
    /// The caller persists returned effects before polling again. A process that has no queued
    /// stdout currently returns `Ok(None)` without performing any I/O.
    ///
    /// # Errors
    /// Returns an error when no process is active, the output source fails, or provider output
    /// violates the bounded framing contract.
    pub fn poll_stdout(&mut self) -> Result<Option<Vec<SessionEffect>>, SupervisorError> {
        let Some(chunk) = self
            .process
            .as_ref()
            .ok_or(SupervisorError::NoActiveProcess)?
            .next_stdout_chunk()?
        else {
            return Ok(None);
        };
        self.offer_output_chunk(&chunk)?;
        let mut effects = Vec::new();
        while self.output.queued_frames() > 0 {
            effects.extend(self.drain_frame().0);
        }
        Ok(Some(effects))
    }

    /// Checks whether the process has exited without reducing terminal state yet.
    ///
    /// A runner must continue polling stdout after this returns an exit code, because a process
    /// implementation can have bounded chunks captured while its readers finish draining.
    ///
    /// # Errors
    /// Returns an error when no process is active or its process tree cannot be inspected.
    pub fn try_exit_code(&self) -> Result<Option<Option<i32>>, SupervisorError> {
        self.process
            .as_ref()
            .ok_or(SupervisorError::NoActiveProcess)?
            .try_exit_code()
            .map_err(SupervisorError::ProcessTree)
    }

    /// Reduces one buffered frame and returns normalized effects for the persistence edge.
    #[must_use]
    pub fn drain_frame(&mut self) -> (Vec<SessionEffect>, Option<ReadDirective>) {
        let (frame, directive) = self.output.take_frame();
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
            permission_binding: None,
            run_id: None,
            lock: self.lock.clone(),
            provider: self.lock.provider.clone(),
            executable: PathBuf::from(&self.lock.canonical_path),
            arguments,
            intent,
            workspace_root: None,
            workspace_access: SandboxWorkspaceAccess::ReadOnly,
        })
    }

    fn apply(&mut self, input: SessionInput) -> Vec<SessionEffect> {
        let transition = self.session.reduce(input);
        self.session = transition.state;
        transition.effects
    }
}

pub use crate::provider_process::ProviderProcess;
