//! Dormant, daemon-owned authority for one reviewed public-provider installation.
//!
//! It is deliberately uncomposed from bootstrap and public IPC. It reserves the exact daemon
//! dependency plan, performs the one private npm effect, and settles the provider lock in the
//! same Gent ledger transaction as the terminal receipt.

use std::sync::{Arc, Mutex};

use gent_drivers::installer::DependencyInstaller;
use gent_ports::{Ledger, PackageInstallPolicy, ProvisionedProviderLockLedger};
use gent_protocol::{
    DependencyActionRequest, DependencyActionResult, DependencyActionState, DependencyPlan,
    DependencyPlanRequest,
};
use gent_runtime::{DependencyActionReceiptClaim, DependencyActionReceiptReservation};

use crate::{
    authority_clock::AuthorityClock,
    dependency_catalog::DependencyCatalog,
    private_provider_provisioning::{
        PrivateProviderProvisioner, PrivateProvisionRequest, PrivateProvisionResult,
        ProvisionReceiptReader, ProvisionedProviderVerifier,
    },
    private_provision_settlement::{
        PrivateProvisionSettlementCoordinator, PrivateProvisionSettlementError,
        PrivateProvisionSettlementState,
    },
};

/// Private provider-provisioning effect owner. Client input contains no package or executable.
#[derive(Clone, Debug)]
pub(crate) struct PrivateProvisioningAuthority<L, I, P, V, R, B, C> {
    plans: DependencyCatalog,
    receipts: DependencyActionReceiptReservation<L>,
    provisioner: PrivateProviderProvisioner<I, P, V, R, B>,
    settlement: PrivateProvisionSettlementCoordinator<L>,
    clock: C,
    serial: Arc<Mutex<()>>,
}

/// A failure while recording the private authority's durable outcome.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PrivateProvisioningAuthorityError {
    #[error(transparent)]
    Runtime(#[from] gent_runtime::RuntimeError),
    #[error(transparent)]
    Settlement(#[from] PrivateProvisionSettlementError),
}

impl<L, I, P, V, R, B, C> PrivateProvisioningAuthority<L, I, P, V, R, B, C> {
    /// Binds all effect authority inputs without registering a public handler.
    #[must_use]
    pub(crate) fn new(
        ledger: L,
        plans: DependencyCatalog,
        provisioner: PrivateProviderProvisioner<I, P, V, R, B>,
        clock: C,
    ) -> Self
    where
        L: Clone,
    {
        Self {
            plans,
            receipts: DependencyActionReceiptReservation::new(ledger.clone()),
            provisioner,
            settlement: PrivateProvisionSettlementCoordinator::new(ledger),
            clock,
            serial: Arc::new(Mutex::new(())),
        }
    }
}

impl<L, I, P, V, R, B, C> PrivateProvisioningAuthority<L, I, P, V, R, B, C>
where
    L: Ledger + ProvisionedProviderLockLedger,
    I: DependencyInstaller,
    P: PackageInstallPolicy,
    V: ProvisionedProviderVerifier,
    R: ProvisionReceiptReader,
    B: crate::private_provider_compatibility::ProvisionedProviderCompatibility,
    C: AuthorityClock,
{
    /// Reserves, verifies, installs, locks, and terminally settles one daemon-issued plan.
    ///
    /// A recovered accepted receipt becomes unprovable without rerunning npm. Any provisioning
    /// error after a durable claim is likewise unprovable: the private effect must never replay.
    ///
    /// # Errors
    /// Returns when the receipt or its terminal settlement cannot be durably recorded.
    pub(crate) fn execute(
        &self,
        request: &DependencyActionRequest,
    ) -> Result<DependencyActionResult, PrivateProvisioningAuthorityError> {
        let _serial = self
            .serial
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let plan = self.plans.plan(DependencyPlanRequest {
            provider: request.provider,
            action: request.action,
        });
        match self.receipts.reserve(request)? {
            DependencyActionReceiptClaim::Claimed(receipt) => {
                let result = if !request.consent_granted {
                    PrivateProvisionResult::ConsentRequired
                } else if !matches_request(request, &plan) {
                    PrivateProvisionResult::Rejected
                } else {
                    self.provisioner
                        .provision(&PrivateProvisionRequest {
                            receipt: receipt.clone(),
                            provider: request.provider,
                            action: request.action,
                            reviewed_plan_digest: request.reviewed_plan_digest.clone(),
                            consent_granted: true,
                            now_unix_seconds: self.clock.now_unix_seconds(),
                        })
                        .unwrap_or(PrivateProvisionResult::Ambiguous)
                };
                self.settle(plan, request, receipt, result)
            }
            DependencyActionReceiptClaim::AcceptedRecovery(receipt) => {
                self.settle(plan, request, receipt, PrivateProvisionResult::Ambiguous)
            }
            DependencyActionReceiptClaim::Terminal(receipt) => Ok(DependencyActionResult {
                plan,
                state: existing_state(&receipt),
                receipt,
                detail: None,
            }),
        }
    }

    fn settle(
        &self,
        plan: DependencyPlan,
        request: &DependencyActionRequest,
        receipt: gent_types::Receipt,
        result: PrivateProvisionResult,
    ) -> Result<DependencyActionResult, PrivateProvisioningAuthorityError> {
        let settlement = self.settlement.settle(
            &PrivateProvisionRequest {
                receipt,
                provider: request.provider,
                action: request.action,
                reviewed_plan_digest: request.reviewed_plan_digest.clone(),
                consent_granted: request.consent_granted,
                now_unix_seconds: self.clock.now_unix_seconds(),
            },
            result,
        )?;
        Ok(DependencyActionResult {
            plan,
            state: settlement_state(settlement.state),
            receipt: settlement.receipt,
            detail: (settlement.state == PrivateProvisionSettlementState::Unprovable)
                .then(|| "private provider installation could not be verified".into()),
        })
    }
}

fn matches_request(request: &DependencyActionRequest, plan: &DependencyPlan) -> bool {
    plan.provider == request.provider
        && plan.action == request.action
        && plan.consent_required
        && plan.reviewed_plan_digest == request.reviewed_plan_digest
}

fn settlement_state(state: PrivateProvisionSettlementState) -> DependencyActionState {
    match state {
        PrivateProvisionSettlementState::Installed => DependencyActionState::Completed,
        PrivateProvisionSettlementState::ConsentRequired => DependencyActionState::ConsentRequired,
        PrivateProvisionSettlementState::Rejected => DependencyActionState::PlanMismatch,
        PrivateProvisionSettlementState::Unprovable => DependencyActionState::Unprovable,
    }
}

fn existing_state(receipt: &gent_types::Receipt) -> DependencyActionState {
    match receipt.status {
        gent_types::ReceiptStatus::Settled => DependencyActionState::Completed,
        gent_types::ReceiptStatus::Unprovable => DependencyActionState::Unprovable,
        gent_types::ReceiptStatus::Accepted | gent_types::ReceiptStatus::Rejected => {
            DependencyActionState::Failed
        }
    }
}

#[cfg(test)]
#[path = "private_provisioning_authority_tests.rs"]
mod tests;
