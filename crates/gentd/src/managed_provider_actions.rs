//! Explicit reviewed updates of installed managed providers into fresh generations.
use crate::{
    authority_clock::AuthorityClock,
    private_provider_provisioning::{PrivateProvisionRequest, PrivateProvisionResult},
    private_provision_settlement::PrivateProvisionSettlementCoordinator,
};
use gent_ports::{
    ApprovedPackageInstall, Ledger, PackageInstallPolicy, ProvisionedProviderLockLedger,
};
use gent_protocol::{
    DependencyAction, DependencyActionRequest, DependencyActionResult, DependencyActionState,
    DependencyPlan, DependencyPlanRequest,
};
use gent_runtime::{DependencyActionReceiptClaim, DependencyActionReceiptReservation};
use gent_types::ReceiptStatus;
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
#[path = "managed_provider_actions/effect.rs"]
mod effect;
#[path = "managed_provider_actions/ports.rs"]
mod ports;
pub(crate) use effect::ManagedProviderEffect;
pub(crate) use ports::{ManagedProvisionPorts, with_operation};

#[derive(Clone, Debug)]
pub(crate) struct ReviewedPackage {
    pub(crate) package: ApprovedPackageInstall,
    pub(crate) release_digest: String,
}

pub(crate) trait ManagedProviderActionPort: Send + Sync {
    fn plan(&self, request: DependencyPlanRequest) -> DependencyPlan;
    fn execute(&self, request: DependencyActionRequest) -> Result<DependencyActionResult, String>;
}

pub(crate) struct ManagedProviderActions<L, P, E, C> {
    ledger: L,
    policy: P,
    effect: E,
    clock: C,
    release_digest: String,
    serial: Arc<Mutex<()>>,
}
impl<L, P, E, C> ManagedProviderActions<L, P, E, C> {
    pub(crate) fn new(ledger: L, policy: P, effect: E, clock: C, release_digest: String) -> Self {
        Self {
            ledger,
            policy,
            effect,
            clock,
            release_digest,
            serial: crate::private_provider_provisioning::effect_serial(),
        }
    }
}
impl<L, P, E, C> ManagedProviderActions<L, P, E, C>
where
    L: Ledger + ProvisionedProviderLockLedger + Clone,
    P: PackageInstallPolicy,
    E: ManagedProviderEffect,
    C: AuthorityClock,
{
    fn review(
        &self,
        request: &DependencyPlanRequest,
    ) -> Result<(DependencyPlan, ReviewedPackage), String> {
        if request.action != DependencyAction::Update {
            return Err("managed repair requires an explicit update".into());
        }
        let package = self
            .policy
            .approved_package(request.provider.as_str(), self.clock.now_unix_seconds())
            .map_err(|e| e.to_string())?;
        if package.provider != request.provider.as_str() {
            return Err("package provider mismatch".into());
        }
        let binding = serde_json::json!({"provider":package.provider,"package":package.package_name,"version":package.version,"integrity":package.integrity,"policy":package.package_policy_digest_sha256,"release":self.release_digest});
        let digest = hex::encode(Sha256::digest(
            serde_json::to_vec(&binding).map_err(|e| e.to_string())?,
        ));
        let instruction = format!(
            "Update {}@{} into a fresh Gent-managed generation. Review identity: {}.",
            package.package_name, package.version, digest
        );
        Ok((
            DependencyPlan::reviewed(request.provider, request.action, instruction, true),
            ReviewedPackage {
                package,
                release_digest: self.release_digest.clone(),
            },
        ))
    }
    fn settle(
        &self,
        request: &PrivateProvisionRequest,
        result: PrivateProvisionResult,
        plan: DependencyPlan,
        state: DependencyActionState,
        detail: Option<String>,
    ) -> Result<DependencyActionResult, String> {
        let settled = PrivateProvisionSettlementCoordinator::new(self.ledger.clone())
            .settle(request, result)
            .map_err(|e| e.to_string())?;
        Ok(DependencyActionResult {
            plan,
            state,
            receipt: settled.receipt,
            detail,
        })
    }
    pub(crate) fn execute(
        &self,
        request: DependencyActionRequest,
    ) -> Result<DependencyActionResult, String> {
        let _serial = self
            .serial
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (plan, review) = self.review(&DependencyPlanRequest {
            provider: request.provider,
            action: request.action,
        })?;
        let claim = DependencyActionReceiptReservation::new(self.ledger.clone())
            .reserve(&request)
            .map_err(|e| e.to_string())?;
        let (receipt, recovery) = match claim {
            DependencyActionReceiptClaim::Claimed(receipt) => (receipt, false),
            DependencyActionReceiptClaim::AcceptedRecovery(receipt) => (receipt, true),
            DependencyActionReceiptClaim::Terminal(receipt) => {
                return Ok(DependencyActionResult {
                    plan,
                    state: match receipt.status {
                        ReceiptStatus::Settled => DependencyActionState::Completed,
                        ReceiptStatus::Unprovable => DependencyActionState::Unprovable,
                        _ => DependencyActionState::Failed,
                    },
                    receipt,
                    detail: None,
                });
            }
        };
        let private = PrivateProvisionRequest {
            receipt,
            provider: request.provider,
            action: request.action,
            reviewed_plan_digest: request.reviewed_plan_digest.clone(),
            consent_granted: request.consent_granted,
            now_unix_seconds: self.clock.now_unix_seconds(),
        };
        if recovery {
            return self.settle(
                &private,
                PrivateProvisionResult::Ambiguous,
                plan,
                DependencyActionState::Unprovable,
                None,
            );
        }
        if !request.consent_granted {
            return self.settle(
                &private,
                PrivateProvisionResult::ConsentRequired,
                plan,
                DependencyActionState::ConsentRequired,
                None,
            );
        }
        if request.reviewed_plan_digest != plan.reviewed_plan_digest {
            return self.settle(
                &private,
                PrivateProvisionResult::Rejected,
                plan,
                DependencyActionState::PlanMismatch,
                None,
            );
        }
        let command = gent_runtime::dependency_action_command(&request);
        match self.effect.provision_reviewed(&private, &command, &review) {
            Ok(PrivateProvisionResult::Installed(installation)) => self.settle(
                &private,
                PrivateProvisionResult::Installed(installation),
                plan,
                DependencyActionState::Completed,
                None,
            ),
            Ok(PrivateProvisionResult::ConsentRequired) => self.settle(
                &private,
                PrivateProvisionResult::ConsentRequired,
                plan,
                DependencyActionState::ConsentRequired,
                None,
            ),
            Ok(PrivateProvisionResult::Rejected) => self.settle(
                &private,
                PrivateProvisionResult::Rejected,
                plan,
                DependencyActionState::Failed,
                None,
            ),
            Err(error) if error.is_definitely_pre_effect() => self.settle(
                &private,
                PrivateProvisionResult::Rejected,
                plan,
                DependencyActionState::Failed,
                Some(error.to_string()),
            ),
            Ok(PrivateProvisionResult::Ambiguous) | Err(_) => self.settle(
                &private,
                PrivateProvisionResult::Ambiguous,
                plan,
                DependencyActionState::Unprovable,
                None,
            ),
        }
    }
}
impl<L, P, E, C> ManagedProviderActionPort for ManagedProviderActions<L, P, E, C>
where
    L: Ledger + ProvisionedProviderLockLedger + Clone + Send + Sync,
    P: PackageInstallPolicy + Send + Sync,
    E: ManagedProviderEffect,
    C: AuthorityClock + Send + Sync,
{
    fn plan(&self, request: DependencyPlanRequest) -> DependencyPlan {
        self.review(&request).map_or_else(
            |_| {
                DependencyPlan::reviewed(
                    request.provider,
                    request.action,
                    "A verified managed package update is unavailable.",
                    false,
                )
            },
            |x| x.0,
        )
    }
    fn execute(&self, request: DependencyActionRequest) -> Result<DependencyActionResult, String> {
        Self::execute(self, request)
    }
}
#[cfg(test)]
#[path = "managed_provider_actions/tests.rs"]
mod tests;
