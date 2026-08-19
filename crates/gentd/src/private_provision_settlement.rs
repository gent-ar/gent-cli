//! Private receipt settlement for verified public-provider provisioning.
//!
//! This module owns no installer, policy, public transport, or bootstrap path. It translates a
//! completed private effect into one atomic Gent-ledger settlement.

use gent_ports::ProvisionedProviderLockLedger;
use gent_types::{Event, Receipt, ReceiptStatus};

use crate::private_provider_provisioning::{PrivateProvisionRequest, PrivateProvisionResult};

/// Durable result of settling one private provisioning attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrivateProvisionSettlement {
    pub(crate) receipt: Receipt,
    pub(crate) state: PrivateProvisionSettlementState,
}

/// Terminal state visible to a future private authority owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrivateProvisionSettlementState {
    Installed,
    ConsentRequired,
    Unprovable,
}

/// Fails closed when the exact accepted receipt can no longer be settled atomically.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PrivateProvisionSettlementError {
    #[error("private provisioning settlement failed: {0}")]
    Ledger(#[from] gent_ports::LedgerError),
}

/// Daemon-owned coordinator that is deliberately uncomposed from public Gent authority.
#[derive(Clone, Debug)]
pub(crate) struct PrivateProvisionSettlementCoordinator<L> {
    ledger: L,
}

impl<L> PrivateProvisionSettlementCoordinator<L> {
    #[must_use]
    pub(crate) const fn new(ledger: L) -> Self {
        Self { ledger }
    }
}

impl<L: ProvisionedProviderLockLedger> PrivateProvisionSettlementCoordinator<L> {
    /// Atomically persists a verified installation and settles the exact durable receipt.
    ///
    /// An ambiguous post-effect result is terminally `Unprovable`: callers must never replay the
    /// external npm action. Consent denial is terminally rejected, requiring a new user action.
    pub(crate) fn settle(
        &self,
        request: &PrivateProvisionRequest,
        result: PrivateProvisionResult,
    ) -> Result<PrivateProvisionSettlement, PrivateProvisionSettlementError> {
        let (state, status, installation) = match result {
            PrivateProvisionResult::Installed(installation) => (
                PrivateProvisionSettlementState::Installed,
                ReceiptStatus::Settled,
                Some(installation),
            ),
            PrivateProvisionResult::ConsentRequired => (
                PrivateProvisionSettlementState::ConsentRequired,
                ReceiptStatus::Rejected,
                None,
            ),
            PrivateProvisionResult::Ambiguous => (
                PrivateProvisionSettlementState::Unprovable,
                ReceiptStatus::Unprovable,
                None,
            ),
        };
        let receipt = self.ledger.settle_provisioned_provider_provision(
            &request.receipt,
            installation.as_deref(),
            status,
            &terminal_event(&request.receipt, state),
        )?;
        Ok(PrivateProvisionSettlement { receipt, state })
    }
}

fn terminal_event(receipt: &Receipt, state: PrivateProvisionSettlementState) -> Event {
    Event {
        cursor: 0,
        event_id: format!("{}:private-provision-terminal", receipt.receipt_id.0),
        receipt_id: receipt.receipt_id.clone(),
        host_epoch: receipt.host_epoch,
        kind: terminal_kind(state).into(),
        payload: serde_json::json!({ "status": receipt_status(state) }),
    }
}

const fn receipt_status(state: PrivateProvisionSettlementState) -> ReceiptStatus {
    match state {
        PrivateProvisionSettlementState::Installed => ReceiptStatus::Settled,
        PrivateProvisionSettlementState::ConsentRequired => ReceiptStatus::Rejected,
        PrivateProvisionSettlementState::Unprovable => ReceiptStatus::Unprovable,
    }
}

const fn terminal_kind(state: PrivateProvisionSettlementState) -> &'static str {
    match state {
        PrivateProvisionSettlementState::Installed => "privateProvisionInstalled",
        PrivateProvisionSettlementState::ConsentRequired => "privateProvisionConsentRequired",
        PrivateProvisionSettlementState::Unprovable => "privateProvisionUnprovable",
    }
}

#[cfg(test)]
#[path = "private_provision_settlement_tests.rs"]
mod tests;
