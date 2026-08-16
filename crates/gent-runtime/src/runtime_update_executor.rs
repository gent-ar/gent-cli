//! Authority-gated staged update effects; unfinished steps are never replayed after restart.

use gent_core::{RuntimeUpdateEvent, RuntimeUpdateIngress, reduce_runtime_update};
use gent_ports::{
    Ledger,
    runtime_update::{
        RuntimeActivation, RuntimeArtifactStager, RuntimeHealthProbe, RuntimeUpdateJournal,
    },
};
use gent_types::{
    HostEpoch, RuntimeStagingReceipt, RuntimeUpdateHandoff, RuntimeUpdateRecord, RuntimeUpdateStage,
};

use crate::{Coordinator, RuntimeError, RuntimeUpdateAuthority, RuntimeUpdatePlan};

/// Result of one staged-update operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeUpdateExecutionResult {
    DeniedObserver,
    Existing(RuntimeUpdateRecord),
    Staged {
        record: RuntimeUpdateRecord,
        receipt: RuntimeStagingReceipt,
    },
    Ready(RuntimeUpdateRecord),
    Failed(RuntimeUpdateRecord),
    HandoffRequested(RuntimeUpdateRecord),
}

/// A single stage, health, or activation effect available only after authority approval.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeUpdateExecution {
    Stage,
    HealthCheck,
    Activate,
}

/// Failure to coordinate a trusted runtime update with durable safety boundaries.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeUpdateExecutionError {
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error(transparent)]
    Ledger(#[from] gent_ports::LedgerError),
    #[error("staging receipt does not match its approved runtime update attempt")]
    ReceiptMismatch,
    #[error("runtime update origin host epoch is no longer active")]
    OriginEpochMismatch,
}

/// Coordinates approved staging, health, and bootstrapper handoff through fakeable ports.
#[derive(Debug)]
pub struct RuntimeUpdateExecutor<L, S, H, A> {
    coordinator: Coordinator<L>,
    journal: L,
    stager: S,
    health: H,
    activation: A,
    authority: RuntimeUpdateAuthority,
}

impl<L, S, H, A> RuntimeUpdateExecutor<L, S, H, A> {
    /// Builds an executor. Shipped observer composition must use `Observer` authority.
    #[must_use]
    pub fn new(
        coordinator: Coordinator<L>,
        journal: L,
        stager: S,
        health: H,
        activation: A,
        authority: RuntimeUpdateAuthority,
    ) -> Self {
        Self {
            coordinator,
            journal,
            stager,
            health,
            activation,
            authority,
        }
    }
}

impl<L, S, H, A> RuntimeUpdateExecutor<L, S, H, A>
where
    L: Ledger + RuntimeUpdateJournal,
    S: RuntimeArtifactStager,
    H: RuntimeHealthProbe,
    A: RuntimeActivation,
{
    /// Performs one stage, health, or activation step; incomplete effects are never retried.
    ///
    /// # Errors
    /// Returns an error when durable state is unavailable or a receipt does not match its attempt.
    pub fn execute(
        &self,
        execution: RuntimeUpdateExecution,
        plan: &RuntimeUpdatePlan,
        host_epoch: HostEpoch,
        staged: Option<&RuntimeStagingReceipt>,
    ) -> Result<RuntimeUpdateExecutionResult, RuntimeUpdateExecutionError> {
        if self.authority != RuntimeUpdateAuthority::Approved {
            return Ok(RuntimeUpdateExecutionResult::DeniedObserver);
        }
        match execution {
            RuntimeUpdateExecution::Stage => self.stage(plan),
            RuntimeUpdateExecution::HealthCheck => self.health_check(plan, host_epoch, staged),
            RuntimeUpdateExecution::Activate => self.activate(plan, staged),
        }
    }

    fn stage(
        &self,
        plan: &RuntimeUpdatePlan,
    ) -> Result<RuntimeUpdateExecutionResult, RuntimeUpdateExecutionError> {
        let current = self.current(&plan.record.attempt_id)?;
        if current != plan.record || current.status.stage != RuntimeUpdateStage::Available {
            return Ok(RuntimeUpdateExecutionResult::Existing(current));
        }
        match self
            .stager
            .stage(&current.attempt_id, &plan.release.payload.artifact)
        {
            Ok(receipt) if matches_receipt(&receipt, &current) => {
                let record = self.checkpoint_staged(current, receipt.clone())?;
                Ok(RuntimeUpdateExecutionResult::Staged { record, receipt })
            }
            Ok(_) | Err(_) => Ok(RuntimeUpdateExecutionResult::Failed(
                self.checkpoint(current, RuntimeUpdateEvent::StagingFailed)?,
            )),
        }
    }

    fn health_check(
        &self,
        plan: &RuntimeUpdatePlan,
        host_epoch: HostEpoch,
        staged: Option<&RuntimeStagingReceipt>,
    ) -> Result<RuntimeUpdateExecutionResult, RuntimeUpdateExecutionError> {
        let current = self.current(&plan.record.attempt_id)?;
        if current.status.stage != RuntimeUpdateStage::Staged {
            return Ok(RuntimeUpdateExecutionResult::Existing(current));
        }
        Self::require_origin_epoch(&current, host_epoch)?;
        let staged = checked_receipt(staged, &current)?.clone();
        let transition = reduce_runtime_update(
            current.status.clone(),
            RuntimeUpdateEvent::HealthCheckStarted,
            None,
        );
        debug_assert_eq!(transition.ingress, RuntimeUpdateIngress::Close);
        let checking = RuntimeUpdateRecord {
            revision: current.revision + 1,
            status: transition.status,
            ..current
        };
        self.coordinator.close_ingress(host_epoch)?;
        self.journal.save_runtime_update(&checking)?;
        let event = if self.health.probe(&staged).is_ok() {
            RuntimeUpdateEvent::HealthCheckPassed
        } else {
            RuntimeUpdateEvent::HealthCheckFailed
        };
        let record = self.checkpoint(checking, event)?;
        Ok(
            if record.status.stage == RuntimeUpdateStage::ReadyToActivate {
                RuntimeUpdateExecutionResult::Ready(record)
            } else {
                RuntimeUpdateExecutionResult::Failed(record)
            },
        )
    }

    fn activate(
        &self,
        plan: &RuntimeUpdatePlan,
        staged: Option<&RuntimeStagingReceipt>,
    ) -> Result<RuntimeUpdateExecutionResult, RuntimeUpdateExecutionError> {
        let current = self.current(&plan.record.attempt_id)?;
        if current.status.stage != RuntimeUpdateStage::ReadyToActivate {
            return Ok(RuntimeUpdateExecutionResult::Existing(current));
        }
        let staged = checked_receipt(staged, &current)?.clone();
        let ingress = self.journal.host_ingress()?;
        Self::require_origin_epoch(&current, ingress.epoch)?;
        let ingress_closed = ingress.mode == gent_ports::IngressMode::Closed;
        let event = if self.activation.activate(&staged).is_ok() {
            RuntimeUpdateEvent::HandoffRequested { ingress_closed }
        } else {
            RuntimeUpdateEvent::ActivationFailed
        };
        let record = self.checkpoint(current, event)?;
        Ok(
            if record.status.stage == RuntimeUpdateStage::HandoffRequested {
                RuntimeUpdateExecutionResult::HandoffRequested(record)
            } else {
                RuntimeUpdateExecutionResult::Failed(record)
            },
        )
    }

    fn current(
        &self,
        attempt_id: &str,
    ) -> Result<RuntimeUpdateRecord, RuntimeUpdateExecutionError> {
        self.journal
            .find_runtime_update(attempt_id)?
            .ok_or_else(|| {
                RuntimeUpdateExecutionError::Ledger(gent_ports::LedgerError::Invariant(
                    "runtime update attempt was not planned".into(),
                ))
            })
    }

    fn checkpoint(
        &self,
        record: RuntimeUpdateRecord,
        event: RuntimeUpdateEvent,
    ) -> Result<RuntimeUpdateRecord, RuntimeUpdateExecutionError> {
        let transition = reduce_runtime_update(record.status.clone(), event, None);
        if transition.ingress == RuntimeUpdateIngress::Close {
            return Err(RuntimeUpdateExecutionError::Ledger(
                gent_ports::LedgerError::Invariant(
                    "runtime update transition requires coordinator ingress closure".into(),
                ),
            ));
        }
        let record = RuntimeUpdateRecord {
            revision: record.revision + 1,
            status: transition.status,
            ..record
        };
        self.journal.save_runtime_update(&record)?;
        Ok(record)
    }

    fn checkpoint_staged(
        &self,
        record: RuntimeUpdateRecord,
        receipt: RuntimeStagingReceipt,
    ) -> Result<RuntimeUpdateRecord, RuntimeUpdateExecutionError> {
        let transition =
            reduce_runtime_update(record.status.clone(), RuntimeUpdateEvent::Staged, None);
        let handoff = RuntimeUpdateHandoff {
            staging_receipt: Some(receipt),
            ..record.handoff.clone()
        };
        let record = RuntimeUpdateRecord {
            revision: record.revision + 1,
            status: transition.status,
            handoff,
            ..record
        };
        self.journal.save_runtime_update(&record)?;
        Ok(record)
    }

    fn require_origin_epoch(
        record: &RuntimeUpdateRecord,
        active: HostEpoch,
    ) -> Result<(), RuntimeUpdateExecutionError> {
        if record.handoff.origin_host_epoch == Some(active) {
            Ok(())
        } else {
            Err(RuntimeUpdateExecutionError::OriginEpochMismatch)
        }
    }
}

fn checked_receipt<'a>(
    staged: Option<&'a RuntimeStagingReceipt>,
    record: &'a RuntimeUpdateRecord,
) -> Result<&'a RuntimeStagingReceipt, RuntimeUpdateExecutionError> {
    let supplied = staged.ok_or(RuntimeUpdateExecutionError::ReceiptMismatch)?;
    let persisted = record
        .handoff
        .staging_receipt
        .as_ref()
        .ok_or(RuntimeUpdateExecutionError::ReceiptMismatch)?;
    if supplied == persisted && matches_receipt(persisted, record) {
        Ok(persisted)
    } else {
        Err(RuntimeUpdateExecutionError::ReceiptMismatch)
    }
}

fn matches_receipt(receipt: &RuntimeStagingReceipt, record: &RuntimeUpdateRecord) -> bool {
    receipt.attempt_id == record.attempt_id
        && receipt.artifact_digest_sha256 == record.artifact_digest_sha256
        && record
            .handoff
            .release
            .as_ref()
            .is_some_and(|release| release.artifact_digest_sha256 == record.artifact_digest_sha256)
}
