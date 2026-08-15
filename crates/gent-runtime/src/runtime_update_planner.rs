//! Authority-gated release planning; observer mode performs no update work.

use gent_core::{
    RuntimeUpdateContext, RuntimeUpdateEvent, RuntimeUpdateIngress, assess_runtime_update,
    reduce_runtime_update,
};
use gent_ports::{
    Ledger,
    runtime_update::{RuntimeReleaseSource, RuntimeUpdateJournal, RuntimeUpdatePortError},
};
use gent_types::{
    HostEpoch, RuntimeUpdateRecord, RuntimeUpdateStage, RuntimeUpdateStatus, SignedRuntimeRelease,
};

use crate::{Coordinator, RuntimeError, RuntimeReleaseTrust, RuntimeReleaseTrustError};

/// Explicit approval boundary for runtime release discovery and ingress changes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RuntimeUpdateAuthority {
    /// Shipped observer behavior: no source access, checkpoints, or ingress changes.
    #[default]
    Observer,
    /// Reserved for a future composition after evidence and authority-transfer approval.
    Approved,
}

/// Immutable inputs for one idempotent release-planning attempt.
#[derive(Clone, Debug)]
pub struct RuntimeUpdateRequest {
    pub attempt_id: String,
    pub host_epoch: HostEpoch,
    pub target: String,
    pub context: RuntimeUpdateContext,
}

/// Result of an authority-gated planning request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeUpdatePlanningResult {
    DeniedObserver,
    Existing(RuntimeUpdateRecord),
    Planned(RuntimeUpdatePlan),
}

/// The verified release and durable checkpoint selected by a new planning attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeUpdatePlan {
    pub release: SignedRuntimeRelease,
    pub record: RuntimeUpdateRecord,
}

/// Error while performing authority-gated planning.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeUpdatePlannerError {
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error(transparent)]
    Ledger(#[from] gent_ports::LedgerError),
    #[error(transparent)]
    Source(#[from] RuntimeUpdatePortError),
    #[error(transparent)]
    Trust(#[from] RuntimeReleaseTrustError),
    #[error("runtime update attempt id and target are required")]
    InvalidRequest,
    #[error("signed runtime release target does not match the requested target")]
    TargetMismatch,
}

/// Selects one signed release and durably records its pure safety disposition.
#[derive(Debug)]
pub struct RuntimeUpdatePlanner<L, S> {
    coordinator: Coordinator<L>,
    journal: L,
    source: S,
    trust: RuntimeReleaseTrust,
    authority: RuntimeUpdateAuthority,
}

impl<L, S> RuntimeUpdatePlanner<L, S> {
    /// Builds a planner. The shipped daemon must use [`RuntimeUpdateAuthority::Observer`].
    #[must_use]
    pub fn new(
        coordinator: Coordinator<L>,
        journal: L,
        source: S,
        trust: RuntimeReleaseTrust,
        authority: RuntimeUpdateAuthority,
    ) -> Self {
        Self {
            coordinator,
            journal,
            source,
            trust,
            authority,
        }
    }
}

impl<L: Ledger + RuntimeUpdateJournal, S: RuntimeReleaseSource> RuntimeUpdatePlanner<L, S> {
    /// Reads, verifies, assesses, and checkpoints one release without staging or activation.
    ///
    /// An existing attempt is returned unchanged. This intentionally does not replay a partially
    /// completed external update action after a restart; a future recovery protocol owns that.
    ///
    /// # Errors
    /// Returns an error when an approved planner cannot safely verify or persist the plan.
    pub fn plan(
        &self,
        request: &RuntimeUpdateRequest,
    ) -> Result<RuntimeUpdatePlanningResult, RuntimeUpdatePlannerError> {
        if self.authority != RuntimeUpdateAuthority::Approved {
            return Ok(RuntimeUpdatePlanningResult::DeniedObserver);
        }
        if request.attempt_id.trim().is_empty() || request.target.trim().is_empty() {
            return Err(RuntimeUpdatePlannerError::InvalidRequest);
        }
        if let Some(existing) = self.journal.find_runtime_update(&request.attempt_id)? {
            return Ok(RuntimeUpdatePlanningResult::Existing(existing));
        }
        let release = self
            .source
            .fetch_release(request.context.selected_channel, &request.target)?;
        self.trust
            .verify_release(&release, request.context.now_unix_seconds)?;
        if release.payload.artifact.target != request.target {
            return Err(RuntimeUpdatePlannerError::TargetMismatch);
        }
        let eligibility = assess_runtime_update(
            &release.payload,
            RuntimeUpdateContext {
                manifest_verified: true,
                ..request.context
            },
        );
        let transition = reduce_runtime_update(
            RuntimeUpdateStatus::default(),
            RuntimeUpdateEvent::Discovered(eligibility),
            Some(&release.payload),
        );
        if transition.ingress == RuntimeUpdateIngress::Close {
            self.coordinator.close_ingress(request.host_epoch)?;
        }
        let record = RuntimeUpdateRecord {
            attempt_id: request.attempt_id.clone(),
            revision: 1,
            artifact_digest_sha256: release.payload.artifact.digest_sha256.clone(),
            status: transition.status,
        };
        self.journal.save_runtime_update(&record)?;
        Ok(RuntimeUpdatePlanningResult::Planned(RuntimeUpdatePlan {
            release,
            record,
        }))
    }

    /// Returns true only for a planned release that is eligible for later staging.
    #[must_use]
    pub const fn can_stage(result: &RuntimeUpdatePlanningResult) -> bool {
        matches!(
            result,
            RuntimeUpdatePlanningResult::Planned(RuntimeUpdatePlan {
                record: RuntimeUpdateRecord {
                    status: gent_types::RuntimeUpdateStatus {
                        stage: RuntimeUpdateStage::Available,
                        ..
                    },
                    ..
                },
                ..
            })
        )
    }
}
