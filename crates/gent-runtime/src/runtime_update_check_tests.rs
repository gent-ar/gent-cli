use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicUsize, Ordering},
};

use ed25519_dalek::{Signer, SigningKey};
use gent_ports::runtime_update::{RuntimeReleaseSource, RuntimeUpdatePortError};
use gent_types::{
    RUNTIME_RELEASE_MANIFEST_VERSION, RuntimeReleaseArtifact, RuntimeReleaseChannel,
    RuntimeReleaseManifest, RuntimeUpdateCheckRequest, RuntimeUpdateCheckState, RuntimeVersion,
    SignedRuntimeRelease,
};

use super::{RuntimeUpdateCheckAuthority, RuntimeUpdateCheckContext, RuntimeUpdateCheckService};
use crate::RuntimeReleaseTrust;

struct Source {
    release: SignedRuntimeRelease,
    calls: AtomicUsize,
}

impl RuntimeReleaseSource for Source {
    fn fetch_release(
        &self,
        _: RuntimeReleaseChannel,
        _: &str,
    ) -> Result<SignedRuntimeRelease, RuntimeUpdatePortError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.release.clone())
    }
}

fn signed_release(key: &SigningKey, version: RuntimeVersion, expiry: u64) -> SignedRuntimeRelease {
    let payload = RuntimeReleaseManifest {
        manifest_version: RUNTIME_RELEASE_MANIFEST_VERSION,
        release_version: version,
        protocol_min: 1,
        protocol_max: 2,
        schema_min: 1,
        schema_max: 1,
        minimum_app_version: RuntimeVersion {
            major: 1,
            minor: 0,
            patch: 0,
        },
        channel: RuntimeReleaseChannel::Stable,
        rollout_percent: 100,
        expires_at_unix_seconds: expiry,
        revoked: false,
        forward_only_schema: false,
        artifact: RuntimeReleaseArtifact {
            target: "test-target".into(),
            archive_name: "gent.tar.gz".into(),
            digest_sha256: "a".repeat(64),
            size_bytes: 1,
        },
    };
    SignedRuntimeRelease {
        key_id: "release-1".into(),
        signature_hex: hex::encode(key.sign(&serde_json::to_vec(&payload).unwrap()).to_bytes()),
        payload,
    }
}

fn context() -> RuntimeUpdateCheckContext {
    RuntimeUpdateCheckContext {
        current_version: RuntimeVersion {
            major: 1,
            minor: 0,
            patch: 0,
        },
        target: "test-target".into(),
        protocol: 1,
        schema: 1,
        app_version: RuntimeVersion {
            major: 1,
            minor: 0,
            patch: 0,
        },
        selected_cohort: true,
    }
}

fn service(
    authority: RuntimeUpdateCheckAuthority,
    expiry: u64,
) -> RuntimeUpdateCheckService<Source> {
    let key = SigningKey::from_bytes(&[9; 32]);
    let source = Source {
        release: signed_release(
            &key,
            RuntimeVersion {
                major: 1,
                minor: 1,
                patch: 0,
            },
            expiry,
        ),
        calls: AtomicUsize::new(0),
    };
    let trust =
        RuntimeReleaseTrust::new(BTreeMap::from([("release-1".into(), key.verifying_key())]));
    RuntimeUpdateCheckService::new(source, trust, context(), authority)
}

#[test]
fn disabled_check_never_reads_a_release_source() {
    let service = service(RuntimeUpdateCheckAuthority::Disabled, 10);
    let report = service.check(
        RuntimeUpdateCheckRequest {
            channel: RuntimeReleaseChannel::Stable,
        },
        1,
    );
    assert_eq!(report.state, RuntimeUpdateCheckState::Unavailable);
    assert_eq!(service.source.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn cached_check_reports_only_a_verified_newer_candidate() {
    let service = service(RuntimeUpdateCheckAuthority::CachedReadOnly, 10);
    let report = service.check(
        RuntimeUpdateCheckRequest {
            channel: RuntimeReleaseChannel::Stable,
        },
        1,
    );
    assert_eq!(report.state, RuntimeUpdateCheckState::Available);
    assert_eq!(report.candidate.unwrap().release_version.patch, 0);
    assert_eq!(service.source.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn cached_check_revalidates_expiry_for_each_request() {
    let service = service(RuntimeUpdateCheckAuthority::CachedReadOnly, 10);
    assert_eq!(
        service
            .check(
                RuntimeUpdateCheckRequest {
                    channel: RuntimeReleaseChannel::Stable
                },
                1
            )
            .state,
        RuntimeUpdateCheckState::Available
    );
    let expired = service.check(
        RuntimeUpdateCheckRequest {
            channel: RuntimeReleaseChannel::Stable,
        },
        11,
    );
    assert_eq!(expired.state, RuntimeUpdateCheckState::Unavailable);
    assert_eq!(
        expired.failure,
        Some(gent_types::RuntimeUpdateFailure::Expired)
    );
}
