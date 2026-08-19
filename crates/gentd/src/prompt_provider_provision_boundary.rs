//! Daemon-only authority for installing the public provider needed by one held prompt.
//!
//! This boundary derives every provider, action, plan, and package value from Gent's durable
//! state. The client confirms only its receipt correlation, held prompt, and reviewed digest.

use std::sync::{Arc, Mutex};

use gent_drivers::installer::DependencyInstaller;
use gent_ports::{
    AgentChatReadLedger, Ledger, PackageInstallPolicy, PrivateProviderPromptProvisionLedger,
    ReceiptClaim,
};
use gent_protocol::{DependencyAction, PromptProviderProvisionFrame, PromptProviderProvisionState};
use gent_runtime::AgentChatReadService;
use gent_types::{Command, ProviderPromptProvisionCommandBinding, Receipt, ReceiptStatus};

use crate::{
    authority_clock::AuthorityClock,
    dependency_catalog::DependencyCatalog,
    private_provider_compatibility::ProvisionedProviderCompatibility,
    private_provider_provisioning::{
        PrivateProviderProvisioner, PrivateProvisionError, PrivateProvisionRequest,
        PrivateProvisionResult, ProvisionReceiptReader, ProvisionedProviderVerifier,
    },
};

mod derive;
mod values;
use values::{Derived, event, public_provider_name, result};

/// Narrow type-erased port used by the facade only after explicit private composition.
pub(crate) trait PromptProviderProvisionPort: Send + Sync {
    /// Confirms a daemon-derived public-provider review for one currently held prompt.
    fn confirm(
        &self,
        frame: PromptProviderProvisionFrame,
    ) -> Result<PromptProviderProvisionFrame, String>;
}

/// Private effect seam which lets this boundary test durable authority without npm.
pub(crate) trait PromptProviderProvisionEffect: Clone + Send + Sync {
    /// Runs the already-reserved, exact prompt-scoped provision command.
    fn provision_prompt(
        &self,
        request: &PrivateProvisionRequest,
        command: &Command,
        binding: &ProviderPromptProvisionCommandBinding,
    ) -> Result<PrivateProvisionResult, PrivateProvisionError>;
}

impl<I, P, V, R, B> PromptProviderProvisionEffect for PrivateProviderProvisioner<I, P, V, R, B>
where
    I: DependencyInstaller + Clone + Send + Sync,
    P: PackageInstallPolicy + Clone + Send + Sync,
    V: ProvisionedProviderVerifier,
    R: ProvisionReceiptReader,
    B: ProvisionedProviderCompatibility,
{
    fn provision_prompt(
        &self,
        request: &PrivateProvisionRequest,
        command: &Command,
        binding: &ProviderPromptProvisionCommandBinding,
    ) -> Result<PrivateProvisionResult, PrivateProvisionError> {
        self.provision_prompt_with_command(request, command, binding)
    }
}

/// Serialized authority for exact public-provider prompt provision confirmations.
#[derive(Clone)]
pub(crate) struct PromptProviderProvisionBoundary<L, P, E, C> {
    ledger: L,
    reads: AgentChatReadService<L>,
    catalog: DependencyCatalog,
    policy: P,
    effect: E,
    clock: C,
    serial: Arc<Mutex<()>>,
}

impl<L, P, E, C> PromptProviderProvisionBoundary<L, P, E, C> {
    /// Binds one daemon ledger, catalog, policy, provisioner, and daemon-owned clock.
    #[must_use]
    pub(crate) fn new(ledger: L, catalog: DependencyCatalog, policy: P, effect: E, clock: C) -> Self
    where
        L: Clone,
    {
        Self {
            reads: AgentChatReadService::new(ledger.clone()),
            catalog,
            ledger,
            policy,
            effect,
            clock,
            serial: Arc::new(Mutex::new(())),
        }
    }
}

impl<L, P, E, C> PromptProviderProvisionBoundary<L, P, E, C>
where
    L: AgentChatReadLedger + Ledger + PrivateProviderPromptProvisionLedger,
    P: PackageInstallPolicy,
    E: PromptProviderProvisionEffect,
    C: AuthorityClock,
{
    /// Runs one receipt-bound prompt-provider confirmation without accepting provider input.
    pub(crate) fn confirm(
        &self,
        frame: PromptProviderProvisionFrame,
    ) -> Result<PromptProviderProvisionFrame, String> {
        frame.validate().map_err(|error| error.to_string())?;
        let _serial = self
            .serial
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let derived =
            derive::request(&self.reads, &self.catalog, &self.policy, &self.clock, frame)?;
        let command = gent_runtime::prompt_provider_provision_command(
            derived.receipt_id.clone(),
            derived.idempotency_key.clone(),
            derived.host_epoch,
            &derived.binding,
        );
        if !derived.binding.prompt.consent_granted {
            return self.reject(
                &command,
                &derived.binding,
                PromptProviderProvisionState::ConsentRequired,
            );
        }
        if derived.binding.prompt.reviewed_plan_digest
            != derived.binding.expected_reviewed_plan_digest
        {
            return self.reject(
                &command,
                &derived.binding,
                PromptProviderProvisionState::PlanMismatch,
            );
        }
        let accepted = event(&command, "accepted", "privatePromptProvisionAccepted");
        match self
            .ledger
            .claim_and_reserve_verified_provider_prompt_provision(
                &command,
                &accepted,
                &derived.binding,
            )
            .map_err(|error| error.to_string())?
        {
            ReceiptClaim::Accepted(receipt) => self.effect_and_settle(&command, &derived, &receipt),
            ReceiptClaim::Existing(receipt) if receipt.status == ReceiptStatus::Accepted => {
                self.unprovable(&command, &derived.binding, &receipt)
            }
            ReceiptClaim::Existing(receipt) if receipt.status == ReceiptStatus::Settled => {
                Ok(result(
                    receipt,
                    &derived.binding,
                    PromptProviderProvisionState::Completed,
                ))
            }
            ReceiptClaim::Existing(receipt) if receipt.status == ReceiptStatus::Unprovable => {
                Ok(result(
                    receipt,
                    &derived.binding,
                    PromptProviderProvisionState::Unprovable,
                ))
            }
            ReceiptClaim::Existing(_) => {
                Err("prompt provision receipt has an invalid terminal state".into())
            }
        }
    }

    fn reject(
        &self,
        command: &Command,
        binding: &ProviderPromptProvisionCommandBinding,
        state: PromptProviderProvisionState,
    ) -> Result<PromptProviderProvisionFrame, String> {
        let kind = match state {
            PromptProviderProvisionState::ConsentRequired => {
                "privatePromptProvisionConsentRequired"
            }
            PromptProviderProvisionState::PlanMismatch => "privatePromptProvisionPlanMismatch",
            _ => return Err("invalid prompt provision rejection state".into()),
        };
        let terminal = event(command, "rejected", kind);
        let receipt = self
            .ledger
            .reject_verified_provider_prompt_provision(command, &terminal, binding)
            .map_err(|error| error.to_string())?;
        Ok(result(receipt, binding, state))
    }

    fn effect_and_settle(
        &self,
        command: &Command,
        derived: &Derived,
        receipt: &Receipt,
    ) -> Result<PromptProviderProvisionFrame, String> {
        let request = PrivateProvisionRequest {
            receipt: receipt.clone(),
            provider: public_provider_name(&derived.binding.prompt.provider)?,
            action: DependencyAction::Install,
            reviewed_plan_digest: derived.binding.expected_reviewed_plan_digest.clone(),
            consent_granted: true,
            now_unix_seconds: self.clock.now_unix_seconds(),
        };
        match self
            .effect
            .provision_prompt(&request, command, &derived.binding)
        {
            Ok(PrivateProvisionResult::Installed(installation)) => {
                let terminal = event(command, "installed", "privatePromptProvisionInstalled");
                let receipt = self
                    .ledger
                    .settle_verified_provider_prompt_provision(
                        command,
                        receipt,
                        &installation,
                        &terminal,
                        &derived.binding,
                    )
                    .map_err(|error| error.to_string())?;
                Ok(result(
                    receipt,
                    &derived.binding,
                    PromptProviderProvisionState::Completed,
                ))
            }
            Ok(_) | Err(_) => self.unprovable(command, &derived.binding, receipt),
        }
    }

    fn unprovable(
        &self,
        command: &Command,
        binding: &ProviderPromptProvisionCommandBinding,
        receipt: &Receipt,
    ) -> Result<PromptProviderProvisionFrame, String> {
        let terminal = event(command, "unprovable", "privatePromptProvisionUnprovable");
        let receipt = self
            .ledger
            .settle_unprovable_provider_prompt_provision(command, receipt, &terminal, binding)
            .map_err(|error| error.to_string())?;
        Ok(result(
            receipt,
            binding,
            PromptProviderProvisionState::Unprovable,
        ))
    }
}

impl<L, P, E, C> PromptProviderProvisionPort for PromptProviderProvisionBoundary<L, P, E, C>
where
    L: AgentChatReadLedger + Ledger + PrivateProviderPromptProvisionLedger,
    P: PackageInstallPolicy + Send + Sync,
    E: PromptProviderProvisionEffect,
    C: AuthorityClock,
{
    fn confirm(
        &self,
        frame: PromptProviderProvisionFrame,
    ) -> Result<PromptProviderProvisionFrame, String> {
        Self::confirm(self, frame)
    }
}

#[cfg(test)]
#[path = "prompt_provider_provision_boundary_tests.rs"]
mod tests;
