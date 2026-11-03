use gent_types::{
    RuntimeReleaseArtifact, RuntimeReleaseChannel, RuntimeReleaseManifest, RuntimeUpdateFailure,
    RuntimeVersion,
};

use super::{RuntimeUpdateContext, RuntimeUpdateEligibility, assess_runtime_update};

fn version(patch: u16) -> RuntimeVersion {
    RuntimeVersion {
        major: 0,
        minor: 2,
        patch,
    }
}

#[test]
fn independent_host_floor_admits_supported_old_hosts_without_tracking_target_version() {
    let mut release = RuntimeReleaseManifest {
        manifest_version: 1,
        release_version: version(5),
        protocol_min: 1,
        protocol_max: 1,
        schema_min: 1,
        schema_max: 26,
        minimum_app_version: version(4),
        channel: RuntimeReleaseChannel::Stable,
        rollout_percent: 100,
        expires_at_unix_seconds: 100,
        revoked: false,
        forward_only_schema: false,
        artifact: RuntimeReleaseArtifact {
            target: "test".into(),
            archive_name: "gent.zip".into(),
            digest_sha256: "a".repeat(64),
            size_bytes: 1,
        },
    };
    for schema in [1, 23, 26] {
        let context = RuntimeUpdateContext {
            protocol: 1,
            schema,
            app_version: version(4),
            selected_channel: RuntimeReleaseChannel::Stable,
            selected_cohort: true,
            manifest_verified: true,
            now_unix_seconds: 1,
        };
        assert_eq!(
            assess_runtime_update(&release, context),
            RuntimeUpdateEligibility::Eligible
        );
        release.minimum_app_version = version(5);
        assert_eq!(
            assess_runtime_update(&release, context),
            RuntimeUpdateEligibility::ReadOnlyUpdateRequired(RuntimeUpdateFailure::IncompatibleApp)
        );
        release.minimum_app_version = version(4);
        assert_eq!(context.schema, schema);
        assert_eq!(context.app_version, version(4));
    }
}
