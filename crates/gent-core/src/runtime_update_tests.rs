use gent_types::{
    RuntimeReleaseArtifact, RuntimeReleaseChannel, RuntimeReleaseManifest, RuntimeUpdateFailure,
    RuntimeUpdateStage, RuntimeUpdateStatus, RuntimeVersion,
};

use super::{
    RuntimeUpdateContext, RuntimeUpdateEligibility, RuntimeUpdateEvent, RuntimeUpdateIngress,
    assess_runtime_update, reduce_runtime_update,
};

fn release() -> RuntimeReleaseManifest {
    RuntimeReleaseManifest {
        manifest_version: 1,
        release_version: RuntimeVersion {
            major: 1,
            minor: 2,
            patch: 3,
        },
        protocol_min: 1,
        protocol_max: 2,
        schema_min: 1,
        schema_max: 2,
        minimum_app_version: RuntimeVersion {
            major: 1,
            minor: 0,
            patch: 0,
        },
        channel: RuntimeReleaseChannel::Stable,
        rollout_percent: 100,
        expires_at_unix_seconds: 20,
        revoked: false,
        forward_only_schema: false,
        artifact: RuntimeReleaseArtifact {
            target: "test".into(),
            archive_name: "gent.tar.gz".into(),
            digest_sha256: "a".repeat(64),
            size_bytes: 1,
        },
    }
}

fn context() -> RuntimeUpdateContext {
    RuntimeUpdateContext {
        protocol: 1,
        schema: 1,
        app_version: RuntimeVersion {
            major: 1,
            minor: 0,
            patch: 0,
        },
        selected_channel: RuntimeReleaseChannel::Stable,
        selected_cohort: true,
        manifest_verified: true,
        now_unix_seconds: 20,
    }
}

#[test]
fn verified_compatible_release_is_eligible_but_channel_and_cohort_can_defer() {
    assert_eq!(
        assess_runtime_update(&release(), context()),
        RuntimeUpdateEligibility::Eligible
    );
    let mut deferred = context();
    deferred.selected_cohort = false;
    assert_eq!(
        assess_runtime_update(&release(), deferred),
        RuntimeUpdateEligibility::Deferred
    );
    deferred.selected_cohort = true;
    deferred.selected_channel = RuntimeReleaseChannel::Beta;
    assert_eq!(
        assess_runtime_update(&release(), deferred),
        RuntimeUpdateEligibility::Deferred
    );
}

#[test]
fn untrusted_expired_revoked_and_bad_digest_releases_cannot_stage() {
    let cases = [
        (
            RuntimeUpdateContext {
                manifest_verified: false,
                ..context()
            },
            RuntimeUpdateFailure::SignatureInvalid,
        ),
        (
            RuntimeUpdateContext {
                now_unix_seconds: 21,
                ..context()
            },
            RuntimeUpdateFailure::Expired,
        ),
    ];
    for (context, failure) in cases {
        assert_eq!(
            assess_runtime_update(&release(), context),
            RuntimeUpdateEligibility::Rejected(failure)
        );
    }
    let mut revoked = release();
    revoked.revoked = true;
    assert_eq!(
        assess_runtime_update(&revoked, context()),
        RuntimeUpdateEligibility::Rejected(RuntimeUpdateFailure::Revoked)
    );
    let mut invalid = release();
    invalid.artifact.digest_sha256.clear();
    assert_eq!(
        assess_runtime_update(&invalid, context()),
        RuntimeUpdateEligibility::Rejected(RuntimeUpdateFailure::InvalidArtifactDigest)
    );
}

#[test]
fn incompatible_contract_requires_read_only_recovery() {
    let cases = [
        (
            RuntimeUpdateContext {
                protocol: 3,
                ..context()
            },
            RuntimeUpdateFailure::IncompatibleProtocol,
        ),
        (
            RuntimeUpdateContext {
                schema: 3,
                ..context()
            },
            RuntimeUpdateFailure::IncompatibleSchema,
        ),
        (
            RuntimeUpdateContext {
                app_version: RuntimeVersion {
                    major: 0,
                    minor: 9,
                    patch: 9,
                },
                ..context()
            },
            RuntimeUpdateFailure::IncompatibleApp,
        ),
    ];
    for (context, failure) in cases {
        assert_eq!(
            assess_runtime_update(&release(), context),
            RuntimeUpdateEligibility::ReadOnlyUpdateRequired(failure)
        );
    }
}

#[test]
fn activation_and_failures_preserve_the_closed_ingress_boundary() {
    let release = release();
    let available = reduce_runtime_update(
        RuntimeUpdateStatus::default(),
        RuntimeUpdateEvent::Discovered(RuntimeUpdateEligibility::Eligible),
        Some(&release),
    );
    let staged =
        reduce_runtime_update(available.status, RuntimeUpdateEvent::Staged, Some(&release));
    let health = reduce_runtime_update(
        staged.status,
        RuntimeUpdateEvent::HealthCheckStarted,
        Some(&release),
    );
    assert_eq!(health.ingress, RuntimeUpdateIngress::Close);
    let ready = reduce_runtime_update(
        health.status,
        RuntimeUpdateEvent::HealthCheckPassed,
        Some(&release),
    );
    let denied = reduce_runtime_update(
        ready.status.clone(),
        RuntimeUpdateEvent::HandoffRequested {
            ingress_closed: false,
        },
        Some(&release),
    );
    assert_eq!(
        denied.status.failure,
        Some(RuntimeUpdateFailure::IngressNotClosed)
    );
    let requested = reduce_runtime_update(
        ready.status,
        RuntimeUpdateEvent::HandoffRequested {
            ingress_closed: true,
        },
        Some(&release),
    );
    assert_eq!(requested.status.stage, RuntimeUpdateStage::HandoffRequested);
    assert_eq!(requested.ingress, RuntimeUpdateIngress::KeepClosed);
    let activated = reduce_runtime_update(
        requested.status,
        RuntimeUpdateEvent::SuccessorConfirmed,
        Some(&release),
    );
    assert_eq!(activated.status.stage, RuntimeUpdateStage::Activated);
    let failed = reduce_runtime_update(
        activated.status,
        RuntimeUpdateEvent::HealthCheckFailed,
        Some(&release),
    );
    assert_eq!(
        failed.status.failure,
        Some(RuntimeUpdateFailure::HealthCheckFailed)
    );
    assert_eq!(failed.ingress, RuntimeUpdateIngress::KeepClosed);
}

#[test]
fn forward_only_update_refuses_rollback_and_invalid_events_are_noops() {
    let release = RuntimeReleaseManifest {
        forward_only_schema: true,
        ..release()
    };
    let available = reduce_runtime_update(
        RuntimeUpdateStatus::default(),
        RuntimeUpdateEvent::Discovered(RuntimeUpdateEligibility::Eligible),
        Some(&release),
    );
    let rollback = reduce_runtime_update(
        available.status,
        RuntimeUpdateEvent::RollbackRequested,
        Some(&release),
    );
    assert_eq!(
        rollback.status.failure,
        Some(RuntimeUpdateFailure::ForwardOnlyRollback)
    );
    assert_eq!(rollback.ingress, RuntimeUpdateIngress::KeepClosed);
    let noop = reduce_runtime_update(
        RuntimeUpdateStatus::default(),
        RuntimeUpdateEvent::Staged,
        None,
    );
    assert!(!noop.changed);
}

#[test]
fn staging_failure_does_not_close_an_unchanged_running_host() {
    let available = reduce_runtime_update(
        RuntimeUpdateStatus::default(),
        RuntimeUpdateEvent::Discovered(RuntimeUpdateEligibility::Eligible),
        Some(&release()),
    );
    let failed = reduce_runtime_update(available.status, RuntimeUpdateEvent::StagingFailed, None);
    assert_eq!(
        failed.status.failure,
        Some(RuntimeUpdateFailure::StagingFailed)
    );
    assert_eq!(failed.ingress, RuntimeUpdateIngress::Unchanged);
}
