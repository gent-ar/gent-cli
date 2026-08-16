//! Durable successor confirmation after an external runtime handoff.
//!
//! This service is intentionally process-free. A platform supervisor owns process replacement;
//! the successor only proves that the closed ledger still names its exact staged release.

use gent_core::{RuntimeUpdateEvent, RuntimeUpdateIngress, reduce_runtime_update};
use gent_ports::{IngressMode, Ledger, runtime_update::RuntimeUpdateJournal};
use gent_types::{
    HostEpoch, RuntimeReleaseIdentity, RuntimeStagingReceipt, RuntimeUpdateRecord,
    RuntimeUpdateStage,
};

use crate::RuntimeUpdateAuthority;

/// Immutable facts a newly started successor must prove before it can settle a handoff.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeUpdateSuccessorRequest {
    pub attempt_id: String,
    pub active_host_epoch: HostEpoch,
    pub release: RuntimeReleaseIdentity,
    pub staging_receipt: RuntimeStagingReceipt,
}

/// The result of an authority-gated successor confirmation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeUpdateSuccessorResult {
    DeniedObserver,
    Confirmed(Box<RuntimeUpdateRecord>),
}

/// Failure to prove that a successor owns an already closed, exact runtime handoff.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeUpdateSuccessorError {
    #[error(transparent)]
    Ledger(#[from] gent_ports::LedgerError),
    #[error("runtime update attempt was not found")]
    UnknownAttempt,
    #[error("runtime update origin host epoch is no longer the active closed epoch")]
    OriginEpochMismatch,
    #[error("runtime update ingress must remain closed until confirmation completes")]
    IngressNotClosed,
    #[error("runtime update release identity does not match the staged handoff")]
    ReleaseMismatch,
    #[error("runtime update staging receipt does not match the staged handoff")]
    ReceiptMismatch,
    #[error("runtime update is not awaiting successor confirmation: {0:?}")]
    InvalidStage(RuntimeUpdateStage),
}

/// Checkpoints core's `SuccessorConfirmed` event only after exact durable handoff validation.
#[derive(Debug)]
pub struct RuntimeUpdateSuccessor<L> {
    journal: L,
    authority: RuntimeUpdateAuthority,
}

impl<L> RuntimeUpdateSuccessor<L> {
    /// Builds a successor confirmation service. Shipped observer composition uses `Observer`.
    #[must_use]
    pub const fn new(journal: L, authority: RuntimeUpdateAuthority) -> Self {
        Self { journal, authority }
    }
}

impl<L: Ledger + RuntimeUpdateJournal> RuntimeUpdateSuccessor<L> {
    /// Proves the successor release and checkpoints activation without opening ingress.
    ///
    /// Repeating an already-confirmed request is read-only and returns its existing checkpoint.
    ///
    /// # Errors
    /// Returns an error when the durable handoff no longer exactly matches the successor.
    pub fn confirm(
        &self,
        request: &RuntimeUpdateSuccessorRequest,
    ) -> Result<RuntimeUpdateSuccessorResult, RuntimeUpdateSuccessorError> {
        if self.authority != RuntimeUpdateAuthority::Approved {
            return Ok(RuntimeUpdateSuccessorResult::DeniedObserver);
        }
        let record = self
            .journal
            .find_runtime_update(&request.attempt_id)?
            .ok_or(RuntimeUpdateSuccessorError::UnknownAttempt)?;
        self.validate(&record, request)?;
        if record.status.stage == RuntimeUpdateStage::Activated {
            return Ok(RuntimeUpdateSuccessorResult::Confirmed(Box::new(record)));
        }
        if record.status.stage != RuntimeUpdateStage::HandoffRequested {
            return Err(RuntimeUpdateSuccessorError::InvalidStage(
                record.status.stage,
            ));
        }
        let transition = reduce_runtime_update(
            record.status.clone(),
            RuntimeUpdateEvent::SuccessorConfirmed,
            None,
        );
        debug_assert_eq!(transition.ingress, RuntimeUpdateIngress::KeepClosed);
        let confirmed = RuntimeUpdateRecord {
            revision: record.revision + 1,
            status: transition.status,
            ..record
        };
        self.journal.save_runtime_update(&confirmed)?;
        Ok(RuntimeUpdateSuccessorResult::Confirmed(Box::new(confirmed)))
    }

    fn validate(
        &self,
        record: &RuntimeUpdateRecord,
        request: &RuntimeUpdateSuccessorRequest,
    ) -> Result<(), RuntimeUpdateSuccessorError> {
        let ingress = self.journal.host_ingress()?;
        if record.handoff.origin_host_epoch != Some(request.active_host_epoch)
            || ingress.epoch != request.active_host_epoch
        {
            return Err(RuntimeUpdateSuccessorError::OriginEpochMismatch);
        }
        if ingress.mode != IngressMode::Closed {
            return Err(RuntimeUpdateSuccessorError::IngressNotClosed);
        }
        if record.handoff.release.as_ref() != Some(&request.release)
            || request.release.artifact_digest_sha256 != record.artifact_digest_sha256
        {
            return Err(RuntimeUpdateSuccessorError::ReleaseMismatch);
        }
        if record.handoff.staging_receipt.as_ref() != Some(&request.staging_receipt)
            || request.staging_receipt.attempt_id != record.attempt_id
            || request.staging_receipt.artifact_digest_sha256 != record.artifact_digest_sha256
        {
            return Err(RuntimeUpdateSuccessorError::ReceiptMismatch);
        }
        Ok(())
    }
}
