//! Pure safety policy for a signed, staged Gent runtime update.

use gent_types::{
    RuntimeReleaseChannel, RuntimeReleaseManifest, RuntimeUpdateFailure, RuntimeUpdateStage,
    RuntimeUpdateStatus, RuntimeVersion,
};

/// Local, verified facts used to assess one release without I/O.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeUpdateContext {
    pub protocol: u16,
    pub schema: u32,
    pub app_version: RuntimeVersion,
    pub selected_channel: RuntimeReleaseChannel,
    pub selected_cohort: bool,
    pub manifest_verified: bool,
    pub now_unix_seconds: u64,
}

/// Whether a release is safe to stage under the current local contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeUpdateEligibility {
    Eligible,
    Deferred,
    ReadOnlyUpdateRequired(RuntimeUpdateFailure),
    Rejected(RuntimeUpdateFailure),
}

/// Pure lifecycle fact from a verified updater orchestrator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeUpdateEvent {
    Discovered(RuntimeUpdateEligibility),
    Staged,
    HealthCheckStarted,
    HealthCheckPassed,
    HealthCheckFailed,
    ActivationRequested { ingress_closed: bool },
    RollbackRequested,
}

/// Required ingress disposition after one updater state transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeUpdateIngress {
    Unchanged,
    Close,
    KeepClosed,
}

/// Complete pure update transition, suitable for isolated tests and orchestration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeUpdateTransition {
    pub status: RuntimeUpdateStatus,
    pub ingress: RuntimeUpdateIngress,
    pub changed: bool,
}

/// Evaluates manifest trust and compatibility before staging may begin.
#[must_use]
pub fn assess_runtime_update(
    manifest: &RuntimeReleaseManifest,
    context: RuntimeUpdateContext,
) -> RuntimeUpdateEligibility {
    if !context.manifest_verified {
        return RuntimeUpdateEligibility::Rejected(RuntimeUpdateFailure::SignatureInvalid);
    }
    if manifest.expires_at_unix_seconds < context.now_unix_seconds {
        return RuntimeUpdateEligibility::Rejected(RuntimeUpdateFailure::Expired);
    }
    if manifest.revoked {
        return RuntimeUpdateEligibility::Rejected(RuntimeUpdateFailure::Revoked);
    }
    if manifest.artifact.digest_sha256.len() != 64 {
        return RuntimeUpdateEligibility::Rejected(RuntimeUpdateFailure::InvalidArtifactDigest);
    }
    if !within_u16(
        context.protocol,
        manifest.protocol_min,
        manifest.protocol_max,
    ) {
        return RuntimeUpdateEligibility::ReadOnlyUpdateRequired(
            RuntimeUpdateFailure::IncompatibleProtocol,
        );
    }
    if !within_u32(context.schema, manifest.schema_min, manifest.schema_max) {
        return RuntimeUpdateEligibility::ReadOnlyUpdateRequired(
            RuntimeUpdateFailure::IncompatibleSchema,
        );
    }
    if context.app_version < manifest.minimum_app_version {
        return RuntimeUpdateEligibility::ReadOnlyUpdateRequired(
            RuntimeUpdateFailure::IncompatibleApp,
        );
    }
    if context.selected_channel != manifest.channel || !context.selected_cohort {
        return RuntimeUpdateEligibility::Deferred;
    }
    RuntimeUpdateEligibility::Eligible
}

/// Reduces updater facts. No binary, ledger, process, network, or clock is accessed.
#[must_use]
pub fn reduce_runtime_update(
    status: RuntimeUpdateStatus,
    event: RuntimeUpdateEvent,
    manifest: Option<&RuntimeReleaseManifest>,
) -> RuntimeUpdateTransition {
    match event {
        RuntimeUpdateEvent::Discovered(eligibility) => discovered(status, eligibility, manifest),
        RuntimeUpdateEvent::Staged if status.stage == RuntimeUpdateStage::Available => transition(
            status,
            RuntimeUpdateStage::Staged,
            None,
            RuntimeUpdateIngress::Unchanged,
        ),
        RuntimeUpdateEvent::HealthCheckStarted if status.stage == RuntimeUpdateStage::Staged => {
            transition(
                status,
                RuntimeUpdateStage::HealthChecking,
                None,
                RuntimeUpdateIngress::Close,
            )
        }
        RuntimeUpdateEvent::HealthCheckPassed
            if status.stage == RuntimeUpdateStage::HealthChecking =>
        {
            transition(
                status,
                RuntimeUpdateStage::ReadyToActivate,
                None,
                RuntimeUpdateIngress::KeepClosed,
            )
        }
        RuntimeUpdateEvent::HealthCheckFailed => {
            failed(status, RuntimeUpdateFailure::HealthCheckFailed)
        }
        RuntimeUpdateEvent::ActivationRequested { ingress_closed }
            if status.stage == RuntimeUpdateStage::ReadyToActivate && ingress_closed =>
        {
            transition(
                status,
                RuntimeUpdateStage::Activated,
                None,
                RuntimeUpdateIngress::KeepClosed,
            )
        }
        RuntimeUpdateEvent::ActivationRequested { .. } => {
            failed(status, RuntimeUpdateFailure::IngressNotClosed)
        }
        RuntimeUpdateEvent::RollbackRequested if status.forward_only_schema => {
            failed(status, RuntimeUpdateFailure::ForwardOnlyRollback)
        }
        RuntimeUpdateEvent::RollbackRequested => transition(
            status,
            RuntimeUpdateStage::RolledBack,
            None,
            RuntimeUpdateIngress::KeepClosed,
        ),
        _ => unchanged(status),
    }
}

fn discovered(
    status: RuntimeUpdateStatus,
    eligibility: RuntimeUpdateEligibility,
    manifest: Option<&RuntimeReleaseManifest>,
) -> RuntimeUpdateTransition {
    match eligibility {
        RuntimeUpdateEligibility::Eligible => manifest.map_or_else(
            || unchanged(status),
            |release| RuntimeUpdateTransition {
                status: RuntimeUpdateStatus {
                    stage: RuntimeUpdateStage::Available,
                    release_version: Some(release.release_version),
                    forward_only_schema: release.forward_only_schema,
                    failure: None,
                },
                ingress: RuntimeUpdateIngress::Unchanged,
                changed: true,
            },
        ),
        RuntimeUpdateEligibility::Deferred => unchanged(status),
        RuntimeUpdateEligibility::ReadOnlyUpdateRequired(failure) => transition(
            status,
            RuntimeUpdateStage::ReadOnlyUpdateRequired,
            Some(failure),
            RuntimeUpdateIngress::Close,
        ),
        RuntimeUpdateEligibility::Rejected(failure) => transition(
            status,
            RuntimeUpdateStage::Failed,
            Some(failure),
            RuntimeUpdateIngress::Unchanged,
        ),
    }
}

fn failed(status: RuntimeUpdateStatus, failure: RuntimeUpdateFailure) -> RuntimeUpdateTransition {
    transition(
        status,
        RuntimeUpdateStage::Failed,
        Some(failure),
        RuntimeUpdateIngress::KeepClosed,
    )
}

fn transition(
    mut status: RuntimeUpdateStatus,
    stage: RuntimeUpdateStage,
    failure: Option<RuntimeUpdateFailure>,
    ingress: RuntimeUpdateIngress,
) -> RuntimeUpdateTransition {
    let changed = status.stage != stage || status.failure != failure;
    status.stage = stage;
    status.failure = failure;
    RuntimeUpdateTransition {
        status,
        ingress,
        changed,
    }
}

const fn unchanged(status: RuntimeUpdateStatus) -> RuntimeUpdateTransition {
    RuntimeUpdateTransition {
        status,
        ingress: RuntimeUpdateIngress::Unchanged,
        changed: false,
    }
}

const fn within_u16(value: u16, minimum: u16, maximum: u16) -> bool {
    value >= minimum && value <= maximum
}

const fn within_u32(value: u32, minimum: u32, maximum: u32) -> bool {
    value >= minimum && value <= maximum
}

#[cfg(test)]
#[path = "runtime_update_tests.rs"]
mod tests;
