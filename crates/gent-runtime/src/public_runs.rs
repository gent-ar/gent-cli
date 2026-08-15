//! Durable public-provider run orchestration. The default observer authority never launches.

use gent_ports::{
    Ledger, PublicProviderResolver, PublicProviderRunError, PublicProviderRunner, RunLease,
    RunLeaseClaim, RunRecord, RunSessionBinding, RunVersionAuthorizer,
};
use gent_protocol::{
    PublicRunInterruptRequest, PublicRunOutcome, PublicRunResponse, PublicRunResumeRequest,
    PublicRunStartRequest,
};
use gent_types::ReceiptId;

use crate::{Coordinator, RuntimeError};

/// Explicit daemon authority required before a public process can be started or signaled.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProviderRunAuthority {
    /// Observer behavior: the process lifecycle is hard-disabled.
    #[default]
    Observer,
    /// A future authority-gated daemon may construct this only after all evidence gates pass.
    PublicDrivers,
}

/// Owns the durable-before-spawn ordering while delegating process effects through one port.
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
    /// Constructs a service. `Observer` is the safe default used by the shipped daemon.
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

    /// Resolves and durably reserves a new root run before invoking the process owner.
    ///
    /// # Errors
    /// Returns an error when durable reservation fails after daemon-owned resolution succeeds.
    pub fn start(&self, request: PublicRunStartRequest) -> Result<PublicRunResponse, RuntimeError> {
        if !self.is_authoritative() {
            return Ok(denied(request.run_id));
        }
        let Ok(lock) = self.resolver.resolve(request.provider.as_str()) else {
            return Ok(denied(request.run_id));
        };
        if self.authorizer.authorize(&lock).is_err() {
            return Ok(denied(request.run_id));
        }
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
        self.coordinator.reserve_public_run(&run, &lock, &lease)?;
        match self.runner.start(&request.run_id, &lock) {
            Ok(()) => Ok(response(request.run_id, PublicRunOutcome::Started)),
            Err(PublicProviderRunError::ProviderChanged) => self.provider_changed(
                &request.run_id,
                &request.coordinator_id,
                request.host_epoch,
                request.provider.as_str(),
            ),
            Err(error) => Err(error.into()),
        }
    }

    /// Reclaims a durable run lease then lets the process owner recheck its immutable lock.
    ///
    /// # Errors
    /// Returns an error when the run or lock is absent or durable lease processing fails.
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
        let lock = self
            .coordinator
            .public_run_lock(&request.run_id)?
            .ok_or_else(|| {
                RuntimeError::Ledger(gent_ports::LedgerError::Invariant(
                    "run has no version lock".into(),
                ))
            })?;
        let lease = RunLease {
            run_id: request.run_id.clone(),
            coordinator_id: request.coordinator_id.clone(),
            host_epoch: request.host_epoch,
        };
        match self.coordinator.claim_run_lease(&lease)? {
            RunLeaseClaim::Contended(_) => {
                Ok(response(request.run_id, PublicRunOutcome::LeaseContended))
            }
            RunLeaseClaim::Acquired(_) | RunLeaseClaim::Recovered { .. } => {
                if self.authorizer.authorize(&lock).is_err() {
                    return Ok(denied(request.run_id));
                }
                match self
                    .runner
                    .resume(&request.run_id, &lock, &session.provider_session_id)
                {
                    Ok(()) => Ok(response(request.run_id, PublicRunOutcome::Resumed)),
                    Err(PublicProviderRunError::ProviderChanged) => self.provider_changed(
                        &request.run_id,
                        &request.coordinator_id,
                        request.host_epoch,
                        &lock.provider,
                    ),
                    Err(error) => Err(error.into()),
                }
            }
        }
    }

    /// Records the provider-native session announced by an owned public-driver process.
    ///
    /// This is an internal daemon lifecycle entry point, deliberately not a protocol request.
    /// A client therefore cannot select the session used by [`Self::resume`].
    ///
    /// # Errors
    /// Returns an error when this daemon is not authoritative or no longer owns the run.
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

    /// Signals only a run whose durable lease still names this coordinator and epoch.
    ///
    /// # Errors
    /// Returns an error only when the owned process cannot be interrupted or storage fails.
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

    fn is_authoritative(&self) -> bool {
        self.authority == ProviderRunAuthority::PublicDrivers
    }

    fn provider_changed(
        &self,
        parent_run_id: &str,
        coordinator_id: &str,
        host_epoch: gent_types::HostEpoch,
        provider: &str,
    ) -> Result<PublicRunResponse, RuntimeError> {
        let child_id = format!("{parent_run_id}:provider-changed:{}", ReceiptId::new().0);
        let Ok(lock) = self.resolver.resolve(provider) else {
            return Ok(response(
                parent_run_id.into(),
                PublicRunOutcome::ProviderChanged,
            ));
        };
        if self.authorizer.authorize(&lock).is_err() {
            return Ok(response(
                parent_run_id.into(),
                PublicRunOutcome::ProviderChanged,
            ));
        }
        self.coordinator.reserve_public_run(
            &RunRecord {
                run_id: child_id.clone(),
                parent_run_id: Some(parent_run_id.into()),
                provider: lock.provider.clone(),
            },
            &lock,
            &RunLease {
                run_id: child_id.clone(),
                coordinator_id: coordinator_id.into(),
                host_epoch,
            },
        )?;
        match self.runner.start(&child_id, &lock) {
            Ok(()) | Err(PublicProviderRunError::ProviderChanged) => {
                Ok(response(child_id, PublicRunOutcome::ProviderChanged))
            }
            Err(error) => Err(error.into()),
        }
    }
}

impl<L: Ledger> Coordinator<L> {
    fn reserve_public_run(
        &self,
        run: &RunRecord,
        lock: &gent_types::RunVersionLock,
        lease: &RunLease,
    ) -> Result<(), RuntimeError> {
        Ok(self.ledger.reserve_run_start(run, lock, lease)?)
    }

    fn public_run_lock(
        &self,
        run_id: &str,
    ) -> Result<Option<gent_types::RunVersionLock>, RuntimeError> {
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

fn denied(run_id: String) -> PublicRunResponse {
    response(run_id, PublicRunOutcome::Denied)
}

fn missing_session() -> RuntimeError {
    RuntimeError::Ledger(gent_ports::LedgerError::Invariant(
        "run has no daemon-owned provider session binding".into(),
    ))
}
