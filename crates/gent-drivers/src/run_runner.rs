//! Public-run adapter over the generic supervisor; it owns no daemon policy or persistence.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use gent_ports::{PublicProviderRunError, PublicProviderRunner};
use gent_types::RunVersionLock;

use crate::buffering::BufferPolicy;
use crate::interrupt::{InterruptEvent, InterruptPolicy};
use crate::launch_spec::arguments;
use crate::lock::LockError;
use crate::session::OutputLimits;
use crate::session::SessionEffect;
use crate::supervisor::{
    LaunchIntent, ProcessLauncher, ProviderProcess, ProviderSupervisor, SupervisorError,
};

#[derive(Debug)]
struct OwnedRun<P: ProviderProcess> {
    supervisor: ProviderSupervisor<P>,
    deadline: Option<InterruptDeadline>,
}

#[derive(Clone, Copy, Debug)]
struct InterruptDeadline {
    event: InterruptEvent,
    at: Instant,
}

/// A process-owning adapter built from an injected launcher and fixed safe stream limits.
#[derive(Debug)]
pub struct DriverRunRunner<L, P: ProviderProcess> {
    launcher: L,
    limits: OutputLimits,
    buffer_policy: BufferPolicy,
    interrupt_policy: InterruptPolicy,
    runs: Mutex<BTreeMap<String, OwnedRun<P>>>,
}

impl<L, P: ProviderProcess> DriverRunRunner<L, P> {
    /// Creates a runner without launching or inspecting any executable.
    #[must_use]
    pub fn new(
        launcher: L,
        limits: OutputLimits,
        buffer_policy: BufferPolicy,
        interrupt_policy: InterruptPolicy,
    ) -> Self {
        Self {
            launcher,
            limits,
            buffer_policy,
            interrupt_policy,
            runs: Mutex::new(BTreeMap::new()),
        }
    }

    /// Sends one pre-encoded frame to a currently owned provider process.
    ///
    /// # Errors
    /// Returns an error when the run is absent or its process cannot accept input.
    pub fn write_frame(&self, run_id: &str, frame: &[u8]) -> Result<(), PublicProviderRunError> {
        lock_runs(&self.runs)
            .get(run_id)
            .ok_or(PublicProviderRunError::NotActive)?
            .supervisor
            .write_frame(frame)
            .map_err(map_error)
    }

    /// Polls one real stdout chunk through the bounded session reducer.
    ///
    /// # Errors
    /// Returns an error when the run is absent or its process/output boundary fails.
    pub fn poll_stdout(
        &self,
        run_id: &str,
    ) -> Result<Option<Vec<SessionEffect>>, PublicProviderRunError> {
        let mut runs = lock_runs(&self.runs);
        let mut settled = false;
        let effects = {
            let run = runs
                .get_mut(run_id)
                .ok_or(PublicProviderRunError::NotActive)?;
            match run.supervisor.poll_stdout().map_err(map_error)? {
                some @ Some(_) => some,
                None => match run.supervisor.try_exit_code().map_err(map_error)? {
                    None => None,
                    Some(code) => {
                        if let some @ Some(_) = run.supervisor.poll_stdout().map_err(map_error)? {
                            some
                        } else {
                            settled = true;
                            Some(run.supervisor.process_exited(code))
                        }
                    }
                },
            }
        };
        if settled {
            let _ = runs.remove(run_id);
        }
        Ok(effects)
    }

    /// Advances one scheduled escalation using caller-supplied monotonic time.
    ///
    /// # Errors
    /// Returns an error when the run is absent or its owned process cannot be signaled.
    pub fn advance_interrupt(
        &self,
        run_id: &str,
        now: Instant,
    ) -> Result<bool, PublicProviderRunError> {
        let mut runs = lock_runs(&self.runs);
        let run = runs
            .get_mut(run_id)
            .ok_or(PublicProviderRunError::NotActive)?;
        let Some(deadline) = run.deadline.filter(|deadline| deadline.at <= now) else {
            return Ok(false);
        };
        let wait = run
            .supervisor
            .interrupt(deadline.event, self.interrupt_policy)
            .map_err(map_error)?;
        run.deadline = next_deadline(deadline.event, now, wait);
        Ok(true)
    }

    /// Ends ownership of a process and cancels any scheduled escalation.
    ///
    /// # Errors
    /// Returns an error when the run is absent.
    pub fn process_exited(
        &self,
        run_id: &str,
        code: Option<i32>,
    ) -> Result<Vec<SessionEffect>, PublicProviderRunError> {
        lock_runs(&self.runs)
            .remove(run_id)
            .ok_or(PublicProviderRunError::NotActive)
            .map(|mut run| run.supervisor.process_exited(code))
    }

    /// Sends the initial interrupt and records its first deadline at caller-supplied time.
    ///
    /// # Errors
    /// Returns an error when the run is absent or its owned process cannot be interrupted.
    pub fn interrupt_at(&self, run_id: &str, now: Instant) -> Result<(), PublicProviderRunError> {
        let mut runs = lock_runs(&self.runs);
        let run = runs
            .get_mut(run_id)
            .ok_or(PublicProviderRunError::NotActive)?;
        let wait = run
            .supervisor
            .interrupt(InterruptEvent::Requested, self.interrupt_policy)
            .map_err(map_error)?;
        if let Some(deadline) = next_deadline(InterruptEvent::Requested, now, wait) {
            run.deadline = Some(deadline);
        }
        Ok(())
    }
}

impl<L, P> PublicProviderRunner for DriverRunRunner<L, P>
where
    L: ProcessLauncher<Process = P>,
    P: ProviderProcess,
{
    fn start(&self, run_id: &str, lock: &RunVersionLock) -> Result<(), PublicProviderRunError> {
        let mut runs = lock_runs(&self.runs);
        if runs.contains_key(run_id) {
            return Err(PublicProviderRunError::Failed("run already active".into()));
        }
        let mut supervisor = ProviderSupervisor::new(lock.clone(), self.limits, self.buffer_policy);
        supervisor
            .spawn(
                &self.launcher,
                arguments(&lock.provider, &LaunchIntent::Start)
                    .map_err(|error| map_spec_error(&error))?,
            )
            .map_err(map_error)?;
        runs.insert(
            run_id.into(),
            OwnedRun {
                supervisor,
                deadline: None,
            },
        );
        Ok(())
    }

    fn resume(
        &self,
        run_id: &str,
        lock: &RunVersionLock,
        session_id: &str,
    ) -> Result<(), PublicProviderRunError> {
        let mut runs = lock_runs(&self.runs);
        if runs.contains_key(run_id) {
            return Err(PublicProviderRunError::Failed("run already active".into()));
        }
        let mut supervisor = ProviderSupervisor::new(lock.clone(), self.limits, self.buffer_policy);
        let intent = LaunchIntent::Resume {
            session_id: session_id.into(),
        };
        supervisor
            .resume(
                &self.launcher,
                arguments(&lock.provider, &intent).map_err(|error| map_spec_error(&error))?,
                session_id.into(),
            )
            .map_err(map_error)?;
        runs.insert(
            run_id.into(),
            OwnedRun {
                supervisor,
                deadline: None,
            },
        );
        Ok(())
    }

    fn interrupt(&self, run_id: &str) -> Result<(), PublicProviderRunError> {
        self.interrupt_at(run_id, Instant::now())
    }
}

fn next_deadline(
    event: InterruptEvent,
    now: Instant,
    wait: Option<u64>,
) -> Option<InterruptDeadline> {
    let event = match event {
        InterruptEvent::Requested => InterruptEvent::InterruptGraceElapsed,
        InterruptEvent::InterruptGraceElapsed => InterruptEvent::TerminateGraceElapsed,
        InterruptEvent::TerminateGraceElapsed | InterruptEvent::Exited => return None,
    };
    wait.map(|milliseconds| InterruptDeadline {
        event,
        at: now + Duration::from_millis(milliseconds),
    })
}

fn lock_runs<P: ProviderProcess>(
    runs: &Mutex<BTreeMap<String, OwnedRun<P>>>,
) -> MutexGuard<'_, BTreeMap<String, OwnedRun<P>>> {
    runs.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn map_error(error: SupervisorError) -> PublicProviderRunError {
    match error {
        SupervisorError::Lock(LockError::ProviderChanged) => {
            PublicProviderRunError::ProviderChanged
        }
        other => PublicProviderRunError::Failed(other.to_string()),
    }
}

fn map_spec_error(error: &crate::launch_spec::LaunchSpecError) -> PublicProviderRunError {
    PublicProviderRunError::Failed(error.to_string())
}
