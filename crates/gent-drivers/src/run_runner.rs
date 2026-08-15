//! Public-run adapter over the generic supervisor; it owns no daemon policy or persistence.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use gent_ports::{PublicProviderRunError, PublicProviderRunner};
use gent_types::RunVersionLock;

use crate::buffering::BufferPolicy;
use crate::interrupt::{InterruptEvent, InterruptPolicy, ProcessTreeControl};
use crate::launch_spec::arguments;
use crate::lock::LockError;
use crate::session::OutputLimits;
use crate::supervisor::{
    LaunchIntent, ProcessLauncher, ProviderProcess, ProviderSupervisor, SupervisorError,
};

/// A process-owning adapter built from an injected launcher and fixed safe stream limits.
#[derive(Debug)]
pub struct DriverRunRunner<L, P: ProcessTreeControl> {
    launcher: L,
    limits: OutputLimits,
    buffer_policy: BufferPolicy,
    interrupt_policy: InterruptPolicy,
    runs: Mutex<BTreeMap<String, ProviderSupervisor<P>>>,
}

impl<L, P: ProcessTreeControl> DriverRunRunner<L, P> {
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
        runs.insert(run_id.into(), supervisor);
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
        runs.insert(run_id.into(), supervisor);
        Ok(())
    }

    fn interrupt(&self, run_id: &str) -> Result<(), PublicProviderRunError> {
        let mut runs = lock_runs(&self.runs);
        let supervisor = runs
            .get_mut(run_id)
            .ok_or(PublicProviderRunError::NotActive)?;
        supervisor
            .interrupt(InterruptEvent::Requested, self.interrupt_policy)
            .map_err(map_error)?;
        Ok(())
    }
}

fn lock_runs<P: ProcessTreeControl>(
    runs: &Mutex<BTreeMap<String, ProviderSupervisor<P>>>,
) -> MutexGuard<'_, BTreeMap<String, ProviderSupervisor<P>>> {
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
