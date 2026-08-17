//! Receipt-backed, lease-fenced orchestration for one fixed read-only Git status effect.
use std::sync::{Arc, Mutex};

use gent_ports::{
    GitExecutor, GitOperationLedger, GitOperationUpdate, GitStatusOperation, GitStatusSummary,
    LeaseClaim, Ledger, ReceiptClaim, WorktreeLease,
};
use gent_types::{
    Event, GitOperationKind, GitOperationPhase, GitOperationRecord, HostEpoch, Receipt, ReceiptId,
    ReceiptStatus,
};

use crate::RuntimeError;
use crate::git_status_events::{
    accepted_event, command_for, receipt_status, summary_payload, terminal_event_id, terminal_kind,
};
/// Explicit receipt, operation, and lease identity for a status request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitStatusRequest {
    pub receipt_id: ReceiptId,
    pub idempotency_key: String,
    pub host_epoch: HostEpoch,
    pub operation: GitOperationRecord,
    pub lease: WorktreeLease,
}

/// Terminal state reported by the narrow Git status service.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitStatusState {
    DeniedObserver,
    Completed,
    Failed,
    Rejected,
    Unprovable,
}

/// Explicit authority required before the daemon may invoke the injected read-only executor.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GitStatusAuthority {
    /// Shipped observer behavior: no receipt, lease, operation, or operating-system effect.
    #[default]
    Observer,
    /// A future reviewed composition may execute only the fixed status operation in this service.
    ApprovedStatus,
}

/// Receipt result. Status output is represented only by an entry count and digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitStatusResult {
    pub state: GitStatusState,
    pub receipt: Option<Receipt>,
    pub summary: Option<GitStatusSummary>,
}

/// Serializes a fixed Git status effect after durable receipt and worktree-lease ownership.
#[derive(Clone, Debug)]
pub struct GitStatusService<L, E> {
    ledger: L,
    executor: E,
    authority: GitStatusAuthority,
    serial: Arc<Mutex<()>>,
}

impl<L, E> GitStatusService<L, E> {
    /// Creates a service. Observer authority makes every request a no-write observer denial.
    #[must_use]
    pub fn new(ledger: L, executor: E, authority: GitStatusAuthority) -> Self {
        Self {
            ledger,
            executor,
            authority,
            serial: Arc::new(Mutex::new(())),
        }
    }
}

impl<L: Ledger + GitOperationLedger + gent_ports::WorkspaceLedger, E: GitExecutor>
    GitStatusService<L, E>
{
    /// Claims a receipt and lease before executing fixed-argv Git status exactly once.
    ///
    /// An accepted receipt seen after a restart is terminally `Unprovable`: an external effect
    /// may have begun, so the operation is never silently replayed.
    ///
    /// # Errors
    /// Returns an error only when durable receipt or lease infrastructure cannot respond.
    pub fn execute(&self, request: &GitStatusRequest) -> Result<GitStatusResult, RuntimeError> {
        if self.authority != GitStatusAuthority::ApprovedStatus {
            return Ok(GitStatusResult {
                state: GitStatusState::DeniedObserver,
                receipt: None,
                summary: None,
            });
        }
        let _serial = self
            .serial
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let command = command_for(request);
        match self
            .ledger
            .claim_command(&command, &accepted_event(&command))?
        {
            ReceiptClaim::Accepted(receipt) => self.execute_claimed(request, &receipt),
            ReceiptClaim::Existing(receipt) if receipt.status == ReceiptStatus::Accepted => {
                self.settle(&receipt, GitStatusState::Unprovable, None)
            }
            ReceiptClaim::Existing(receipt) => self.existing(receipt),
        }
    }

    fn execute_claimed(
        &self,
        request: &GitStatusRequest,
        receipt: &Receipt,
    ) -> Result<GitStatusResult, RuntimeError> {
        if !valid_request(request) {
            return self.settle(receipt, GitStatusState::Rejected, None);
        }
        let Some(worktree) = self.ledger.find_worktree(&request.operation.worktree_id)? else {
            return self.settle(receipt, GitStatusState::Rejected, None);
        };
        if matches!(
            self.ledger.claim_worktree_lease(&request.lease)?,
            LeaseClaim::Contended(_)
        ) {
            return self.settle(receipt, GitStatusState::Rejected, None);
        }
        if self
            .ledger
            .create_git_operation(&request.operation)
            .is_err()
        {
            return self.settle(receipt, GitStatusState::Rejected, None);
        }
        if !self.transition(
            &request.operation.operation_id,
            GitOperationPhase::Requested,
            GitOperationPhase::Running,
        )? {
            return self.settle(receipt, GitStatusState::Unprovable, None);
        }
        let outcome = self.executor.status(&GitStatusOperation {
            canonical_worktree_path: worktree.canonical_path,
        });
        if let Ok(summary) = outcome {
            if !self.transition(
                &request.operation.operation_id,
                GitOperationPhase::Running,
                GitOperationPhase::Succeeded,
            )? {
                return self.settle(receipt, GitStatusState::Unprovable, None);
            }
            self.settle(receipt, GitStatusState::Completed, Some(summary))
        } else {
            if !self.transition(
                &request.operation.operation_id,
                GitOperationPhase::Running,
                GitOperationPhase::Failed,
            )? {
                return self.settle(receipt, GitStatusState::Unprovable, None);
            }
            self.settle(receipt, GitStatusState::Failed, None)
        }
    }

    fn existing(&self, receipt: Receipt) -> Result<GitStatusResult, RuntimeError> {
        let state = match receipt.status {
            ReceiptStatus::Settled => GitStatusState::Completed,
            ReceiptStatus::Unprovable => GitStatusState::Unprovable,
            ReceiptStatus::Rejected | ReceiptStatus::Accepted => GitStatusState::Failed,
        };
        let summary = if state == GitStatusState::Completed {
            self.summary_for(&receipt)?
        } else {
            None
        };
        Ok(GitStatusResult {
            state,
            receipt: Some(receipt),
            summary,
        })
    }

    fn transition(
        &self,
        operation_id: &str,
        expected: GitOperationPhase,
        next: GitOperationPhase,
    ) -> Result<bool, RuntimeError> {
        Ok(matches!(
            self.ledger
                .replace_git_operation_phase(operation_id, expected, next)?,
            GitOperationUpdate::Applied(_)
        ))
    }

    fn settle(
        &self,
        receipt: &Receipt,
        state: GitStatusState,
        summary: Option<GitStatusSummary>,
    ) -> Result<GitStatusResult, RuntimeError> {
        let status = receipt_status(state);
        let terminal = Event {
            cursor: 0,
            event_id: terminal_event_id(&receipt.receipt_id),
            receipt_id: receipt.receipt_id.clone(),
            host_epoch: receipt.host_epoch,
            kind: terminal_kind(state).into(),
            payload: summary_payload(summary.as_ref()),
        };
        let receipt = self
            .ledger
            .settle_receipt(&receipt.idempotency_key, status, &terminal)?;
        Ok(GitStatusResult {
            state,
            receipt: Some(receipt),
            summary,
        })
    }

    fn summary_for(&self, receipt: &Receipt) -> Result<Option<GitStatusSummary>, RuntimeError> {
        let Some(event) = self
            .ledger
            .find_event(&terminal_event_id(&receipt.receipt_id))?
        else {
            return Ok(None);
        };
        let entry_count = event
            .payload
            .get("entryCount")
            .and_then(serde_json::Value::as_u64);
        let digest = event
            .payload
            .get("outputDigestSha256")
            .and_then(serde_json::Value::as_str);
        Ok(entry_count
            .zip(digest)
            .map(|(entry_count, output_digest_sha256)| GitStatusSummary {
                entry_count: u32::try_from(entry_count).unwrap_or(u32::MAX),
                output_digest_sha256: output_digest_sha256.into(),
            }))
    }
}

fn valid_request(request: &GitStatusRequest) -> bool {
    request.operation.kind == GitOperationKind::Status
        && request.operation.phase == GitOperationPhase::Requested
        && request.operation.worktree_id == request.lease.worktree_id
        && request.operation.run_id == request.lease.run_id
        && request.host_epoch == request.lease.host_epoch
}
