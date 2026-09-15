use crate::{Coordinator, RuntimeError};
use gent_ports::{
    Ledger, PublicProviderResolver, PublicProviderRunError, PublicProviderRunner, RunLease,
    RunLeaseClaim, RunRecord, RunSessionBinding, RunVersionAuthorizer,
};
use gent_protocol::{
    PublicRunInterruptRequest, PublicRunOutcome, PublicRunResponse, PublicRunResumeRequest,
    PublicRunStartRequest,
};
use gent_types::RunVersionLock;
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProviderRunAuthority {
    #[default]
    Observer,
    PublicDrivers,
    PrivateClaurstBridge,
}
#[derive(Debug)]
pub struct PublicRunService<L, D, A, R> {
    coordinator: Coordinator<L>,
    runner: D,
    authorizer: A,
    resolver: R,
    authority: ProviderRunAuthority,
}
impl<L, D, A, R> PublicRunService<L, D, A, R>
where
    L: Ledger,
    D: PublicProviderRunner,
    A: RunVersionAuthorizer,
    R: PublicProviderResolver,
{
    #[must_use]
    pub fn new(
        coordinator: Coordinator<L>,
        runner: D,
        authorizer: A,
        resolver: R,
        authority: ProviderRunAuthority,
    ) -> Self {
        Self {
            coordinator,
            runner,
            authorizer,
            resolver,
            authority,
        }
    }

    pub fn start(&self, request: PublicRunStartRequest) -> Result<PublicRunResponse, RuntimeError> {
        if !self.is_authoritative() {
            return Ok(denied(request.run_id));
        }
        let Some(lock) = self.authorized(request.provider.as_str()) else {
            return Ok(denied(request.run_id));
        };
        let run = RunRecord {
            run_id: request.run_id.clone(),
            parent_run_id: None,
            provider: request.provider.as_str().into(),
        };
        let lease = RunLease {
            run_id: request.run_id.clone(),
            coordinator_id: request.coordinator_id.clone(),
            host_epoch: request.host_epoch,
        };
        if matches!(
            self.coordinator
                .reserve_or_activate_public_run(&run, &lock, &lease)?,
            RunLeaseClaim::Contended(_)
        ) {
            return Ok(response(request.run_id, PublicRunOutcome::LeaseContended));
        }
        launched(
            request.run_id.clone(),
            self.runner.start(&request.run_id, &lock),
            PublicRunOutcome::Started,
        )
    }

    pub fn start_or_resume(
        &self,
        request: PublicRunStartRequest,
    ) -> Result<PublicRunResponse, RuntimeError> {
        if !self.is_authoritative() {
            return Ok(denied(request.run_id));
        }
        if self
            .coordinator
            .public_run_session(&request.run_id)?
            .is_some()
        {
            return self.resume(PublicRunResumeRequest {
                run_id: request.run_id,
                coordinator_id: request.coordinator_id,
                host_epoch: request.host_epoch,
            });
        }
        self.start(request)
    }

    pub fn resume(
        &self,
        request: PublicRunResumeRequest,
    ) -> Result<PublicRunResponse, RuntimeError> {
        if !self.is_authoritative() {
            return Ok(denied(request.run_id));
        }
        let Some(session) = self.coordinator.public_run_session(&request.run_id)? else {
            return Err(missing_session());
        };
        let provider = self
            .coordinator
            .public_run_lock(&request.run_id)?
            .ok_or_else(|| {
                RuntimeError::Ledger(gent_ports::LedgerError::Invariant(
                    "run has no version lock".into(),
                ))
            })?
            .provider;
        let Some(lock) = self.authorized(&provider) else {
            return Ok(denied(request.run_id));
        };
        let lease = RunLease {
            run_id: request.run_id.clone(),
            coordinator_id: request.coordinator_id.clone(),
            host_epoch: request.host_epoch,
        };
        if matches!(
            self.coordinator
                .ledger
                .activate_existing_run_start(&lock, &lease)?,
            RunLeaseClaim::Contended(_)
        ) {
            return Ok(response(request.run_id, PublicRunOutcome::LeaseContended));
        }
        launched(
            request.run_id.clone(),
            self.runner
                .resume(&request.run_id, &lock, &session.provider_session_id),
            PublicRunOutcome::Resumed,
        )
    }

    pub fn launched_executable_is_current(&self, run_id: &str) -> Result<bool, RuntimeError> {
        let Some(stored) = self.coordinator.public_run_lock(run_id)? else {
            return Ok(false);
        };
        Ok(self.authorized(&stored.provider).as_ref() == Some(&stored))
    }

    pub fn record_provider_session(
        &self,
        run_id: String,
        coordinator_id: &str,
        host_epoch: gent_types::HostEpoch,
        provider_session_id: String,
    ) -> Result<(), RuntimeError> {
        if !self.is_authoritative() {
            return Err(RuntimeError::Ledger(gent_ports::LedgerError::Invariant(
                "observer mode cannot record public provider sessions".into(),
            )));
        }
        let owned = self
            .coordinator
            .public_run_lease(&run_id)?
            .is_some_and(|lease| {
                lease.coordinator_id == coordinator_id && lease.host_epoch == host_epoch
            });
        if !owned {
            return Err(RuntimeError::Ledger(gent_ports::LedgerError::Invariant(
                "provider session reporter does not own the run".into(),
            )));
        }
        self.coordinator
            .save_public_run_session(&RunSessionBinding {
                run_id,
                provider_session_id,
            })
    }

    pub fn retire_provider_session(
        &self,
        run_id: &str,
        host_epoch: gent_types::HostEpoch,
    ) -> Result<(), RuntimeError> {
        let Some(binding) = self.coordinator.public_run_session(run_id)? else {
            return Ok(());
        };
        Ok(self
            .coordinator
            .ledger
            .retire_run_session_binding(&binding, host_epoch)?)
    }

    pub fn interrupt(
        &self,
        request: PublicRunInterruptRequest,
    ) -> Result<PublicRunResponse, RuntimeError> {
        if !self.is_authoritative() {
            return Ok(denied(request.run_id));
        }
        let lease = self.coordinator.public_run_lease(&request.run_id)?;
        let owned = lease.is_some_and(|lease| {
            lease.coordinator_id == request.coordinator_id && lease.host_epoch == request.host_epoch
        });
        if !owned {
            return Ok(response(request.run_id, PublicRunOutcome::LeaseContended));
        }
        self.runner.interrupt(&request.run_id)?;
        Ok(response(request.run_id, PublicRunOutcome::Interrupted))
    }

    fn authorized(&self, provider: &str) -> Option<RunVersionLock> {
        let lock = self.resolver.resolve(provider).ok()?;
        (lock.provider == provider && self.authorizer.authorize(&lock).is_ok()).then_some(lock)
    }

    fn is_authoritative(&self) -> bool {
        self.authority == ProviderRunAuthority::PublicDrivers
    }
}
impl<L: Ledger> Coordinator<L> {
    fn reserve_or_activate_public_run(
        &self,
        run: &RunRecord,
        lock: &RunVersionLock,
        lease: &RunLease,
    ) -> Result<RunLeaseClaim, RuntimeError> {
        if self.ledger.find_run(&run.run_id)?.is_some() {
            return Ok(self.ledger.activate_existing_run_start(lock, lease)?);
        }
        self.ledger.reserve_run_start(run, lock, lease)?;
        Ok(RunLeaseClaim::Acquired(lease.clone()))
    }

    fn public_run_lock(&self, run_id: &str) -> Result<Option<RunVersionLock>, RuntimeError> {
        Ok(self.ledger.find_run_version_lock(run_id)?)
    }

    fn public_run_session(&self, run_id: &str) -> Result<Option<RunSessionBinding>, RuntimeError> {
        Ok(self.ledger.find_run_session_binding(run_id)?)
    }

    fn save_public_run_session(&self, binding: &RunSessionBinding) -> Result<(), RuntimeError> {
        Ok(self.ledger.save_run_session_binding(binding)?)
    }

    fn public_run_lease(&self, run_id: &str) -> Result<Option<RunLease>, RuntimeError> {
        Ok(self.ledger.find_run_lease(run_id)?)
    }
}

const fn response(run_id: String, outcome: PublicRunOutcome) -> PublicRunResponse {
    PublicRunResponse { run_id, outcome }
}

fn launched(
    run_id: String,
    launch: Result<(), PublicProviderRunError>,
    outcome: PublicRunOutcome,
) -> Result<PublicRunResponse, RuntimeError> {
    match launch {
        Ok(()) => Ok(response(run_id, outcome)),
        Err(PublicProviderRunError::ProviderChanged) => Ok(denied(run_id)),
        Err(error) => Err(error.into()),
    }
}

fn denied(run_id: String) -> PublicRunResponse {
    response(run_id, PublicRunOutcome::Denied)
}

fn missing_session() -> RuntimeError {
    RuntimeError::Ledger(gent_ports::LedgerError::Invariant(
        "run has no daemon-owned provider session binding".into(),
    ))
}
