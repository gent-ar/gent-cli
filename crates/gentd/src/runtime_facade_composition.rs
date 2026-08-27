//! Typed capability-profile composition for the single daemon ledger and coordinator.

use std::path::{Path, PathBuf};

use gent_runtime::Coordinator;
use gent_runtime::catalog::{RuntimeCapabilityProfile, declared_capabilities_with_profiles};
use gent_store::SqliteLedger;

use super::RuntimeFacade;
use crate::compatibility_assessment::CompatibilityAssessment;
use crate::runtime_update_config::DaemonRuntimeUpdateChecks;

/// The one already-open durable state shared by future daemon compositions.
///
/// Opening this state prepares only current local storage and its profile-derived capabilities.
/// It does not discover or launch a provider, and does not select any authority profile.
#[derive(Debug)]
pub(crate) struct DaemonCompositionState {
    pub(super) data_dir: PathBuf,
    pub(super) ledger: SqliteLedger,
    pub(super) coordinator: Coordinator<SqliteLedger>,
    pub(super) capability_profile: RuntimeCapabilityProfile,
    pub(super) compatibility: CompatibilityAssessment,
}

impl DaemonCompositionState {
    /// Opens the single Gent ledger and coordinator from one typed authority profile.
    ///
    /// # Errors
    /// Returns an error when the local durable state cannot open.
    pub(crate) fn open(
        data_dir: &Path,
        capability_profile: &RuntimeCapabilityProfile,
        compatibility: CompatibilityAssessment,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let capabilities = declared_capabilities_with_profiles(capability_profile);
        let ledger = SqliteLedger::open(data_dir.join("gent.db"))?;
        crate::permission_workspace::ensure(&ledger, data_dir)?;
        let coordinator = Coordinator::new(ledger.clone(), capabilities);
        Ok(Self {
            data_dir: data_dir.into(),
            ledger,
            coordinator,
            capability_profile: capability_profile.clone(),
            compatibility,
        })
    }

    pub(crate) fn fence_unclean_predecessor(&self) -> Result<(), Box<dyn std::error::Error>> {
        let epoch = self.coordinator.status()?.host_epoch;
        self.coordinator.close_ingress(epoch)?;
        self.coordinator.fence_and_open(epoch)?;
        Ok(())
    }

    #[must_use]
    pub(crate) fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    #[must_use]
    pub(crate) fn ledger(&self) -> &SqliteLedger {
        &self.ledger
    }

    #[must_use]
    pub(crate) fn coordinator(&self) -> &Coordinator<SqliteLedger> {
        &self.coordinator
    }

    #[must_use]
    pub(crate) fn capability_profile(&self) -> &RuntimeCapabilityProfile {
        &self.capability_profile
    }

    #[must_use]
    pub(crate) fn compatibility(&self) -> &CompatibilityAssessment {
        &self.compatibility
    }
}

/// Builds the current observer-only daemon composition.
///
/// # Errors
/// Returns an error when required local storage cannot open.
#[cfg(test)]
pub(crate) fn build_runtime(
    data_dir: &Path,
    capability_profile: &RuntimeCapabilityProfile,
    compatibility: CompatibilityAssessment,
) -> Result<RuntimeFacade, Box<dyn std::error::Error>> {
    build_runtime_with_update_checks(data_dir, capability_profile, compatibility, None)
}

/// Builds the daemon with explicit update checks and durable-chat read profiles.
///
/// # Errors
/// Returns an error when required local storage cannot open.
pub(crate) fn build_runtime_with_update_checks(
    data_dir: &Path,
    capability_profile: &RuntimeCapabilityProfile,
    compatibility: CompatibilityAssessment,
    runtime_update_checks: Option<DaemonRuntimeUpdateChecks>,
) -> Result<RuntimeFacade, Box<dyn std::error::Error>> {
    let state = DaemonCompositionState::open(data_dir, capability_profile, compatibility)?;
    RuntimeFacade::from_state(state, runtime_update_checks)
}
