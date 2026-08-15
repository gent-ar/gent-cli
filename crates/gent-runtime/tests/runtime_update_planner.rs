use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use ed25519_dalek::{Signer, SigningKey};
use gent_core::RuntimeUpdateContext;
use gent_ports::runtime_update::{
    RuntimeReleaseSource, RuntimeUpdateJournal, RuntimeUpdatePortError,
};
use gent_ports::{IngressMode, Ledger};
use gent_runtime::{
    Coordinator, RuntimeReleaseTrust, RuntimeUpdateAuthority, RuntimeUpdatePlanner,
    RuntimeUpdatePlanningResult, RuntimeUpdateRequest,
};
use gent_store::SqliteLedger;
use gent_types::{
    CapabilitySet, HostEpoch, RUNTIME_RELEASE_MANIFEST_VERSION, RuntimeReleaseArtifact,
    RuntimeReleaseChannel, RuntimeReleaseManifest, RuntimeUpdateStage, RuntimeVersion,
    SignedRuntimeRelease,
};

#[derive(Clone, Debug)]
struct Source {
    release: SignedRuntimeRelease,
    calls: Arc<Mutex<u8>>,
}

impl RuntimeReleaseSource for Source {
    fn fetch_release(
        &self,
        _: RuntimeReleaseChannel,
        _: &str,
    ) -> Result<SignedRuntimeRelease, RuntimeUpdatePortError> {
        *self.calls.lock().unwrap() += 1;
        Ok(self.release.clone())
    }
}

fn signed_release(key: &SigningKey, minimum_app: RuntimeVersion) -> SignedRuntimeRelease {
    let payload = RuntimeReleaseManifest {
        manifest_version: RUNTIME_RELEASE_MANIFEST_VERSION,
        release_version: RuntimeVersion {
            major: 2,
            minor: 0,
            patch: 0,
        },
        protocol_min: 1,
        protocol_max: 1,
        schema_min: 1,
        schema_max: 1,
        minimum_app_version: minimum_app,
        channel: RuntimeReleaseChannel::Stable,
        rollout_percent: 100,
        expires_at_unix_seconds: 100,
        revoked: false,
        forward_only_schema: false,
        artifact: RuntimeReleaseArtifact {
            target: "aarch64-apple-darwin".into(),
            archive_name: "gent.tar.gz".into(),
            digest_sha256: "c".repeat(64),
            size_bytes: 1,
        },
    };
    SignedRuntimeRelease {
        key_id: "release-1".into(),
        signature_hex: hex::encode(key.sign(&serde_json::to_vec(&payload).unwrap()).to_bytes()),
        payload,
    }
}

fn request() -> RuntimeUpdateRequest {
    RuntimeUpdateRequest {
        attempt_id: "attempt-1".into(),
        host_epoch: HostEpoch(1),
        target: "aarch64-apple-darwin".into(),
        context: RuntimeUpdateContext {
            protocol: 1,
            schema: 1,
            app_version: RuntimeVersion {
                major: 1,
                minor: 0,
                patch: 0,
            },
            selected_channel: RuntimeReleaseChannel::Stable,
            selected_cohort: true,
            manifest_verified: false,
            now_unix_seconds: 10,
        },
    }
}

fn planner(
    authority: RuntimeUpdateAuthority,
    source: Source,
    ledger: SqliteLedger,
    key: &SigningKey,
) -> RuntimeUpdatePlanner<SqliteLedger, Source> {
    let trust =
        RuntimeReleaseTrust::new(BTreeMap::from([("release-1".into(), key.verifying_key())]));
    RuntimeUpdatePlanner::new(
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger,
        source,
        trust,
        authority,
    )
}

#[test]
fn observer_planner_never_fetches_writes_or_closes_ingress() {
    let key = SigningKey::from_bytes(&[4; 32]);
    let calls = Arc::new(Mutex::new(0));
    let ledger = SqliteLedger::in_memory().unwrap();
    let planner = planner(
        RuntimeUpdateAuthority::Observer,
        Source {
            release: signed_release(
                &key,
                RuntimeVersion {
                    major: 1,
                    minor: 0,
                    patch: 0,
                },
            ),
            calls: calls.clone(),
        },
        ledger.clone(),
        &key,
    );
    assert_eq!(
        planner.plan(&request()).unwrap(),
        RuntimeUpdatePlanningResult::DeniedObserver
    );
    assert_eq!(*calls.lock().unwrap(), 0);
    assert_eq!(ledger.host_ingress().unwrap().mode, IngressMode::Open);
}

#[test]
fn approved_planner_verifies_and_checkpoints_an_eligible_release_once() {
    let key = SigningKey::from_bytes(&[4; 32]);
    let calls = Arc::new(Mutex::new(0));
    let ledger = SqliteLedger::in_memory().unwrap();
    let planner = planner(
        RuntimeUpdateAuthority::Approved,
        Source {
            release: signed_release(
                &key,
                RuntimeVersion {
                    major: 1,
                    minor: 0,
                    patch: 0,
                },
            ),
            calls: calls.clone(),
        },
        ledger.clone(),
        &key,
    );
    let first = planner.plan(&request()).unwrap();
    assert!(RuntimeUpdatePlanner::<SqliteLedger, Source>::can_stage(
        &first
    ));
    assert_eq!(*calls.lock().unwrap(), 1);
    assert_eq!(
        ledger
            .find_runtime_update("attempt-1")
            .unwrap()
            .unwrap()
            .status
            .stage,
        RuntimeUpdateStage::Available
    );
    assert!(matches!(
        planner.plan(&request()).unwrap(),
        RuntimeUpdatePlanningResult::Existing(_)
    ));
    assert_eq!(*calls.lock().unwrap(), 1);
}

#[test]
fn incompatible_approved_plan_closes_ingress_before_checkpointing_read_only_state() {
    let key = SigningKey::from_bytes(&[4; 32]);
    let ledger = SqliteLedger::in_memory().unwrap();
    let planner = planner(
        RuntimeUpdateAuthority::Approved,
        Source {
            release: signed_release(
                &key,
                RuntimeVersion {
                    major: 2,
                    minor: 0,
                    patch: 0,
                },
            ),
            calls: Arc::new(Mutex::new(0)),
        },
        ledger.clone(),
        &key,
    );
    let result = planner.plan(&request()).unwrap();
    assert!(!RuntimeUpdatePlanner::<SqliteLedger, Source>::can_stage(
        &result
    ));
    assert_eq!(ledger.host_ingress().unwrap().mode, IngressMode::Closed);
    assert_eq!(
        ledger
            .find_runtime_update("attempt-1")
            .unwrap()
            .unwrap()
            .status
            .stage,
        RuntimeUpdateStage::ReadOnlyUpdateRequired
    );
}
