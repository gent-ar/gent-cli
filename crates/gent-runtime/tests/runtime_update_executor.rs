use std::sync::{Arc, Mutex};

use gent_ports::runtime_update::{
    RuntimeActivation, RuntimeArtifactStager, RuntimeHealthProbe, RuntimeUpdateJournal,
    RuntimeUpdatePortError,
};
use gent_ports::{IngressMode, Ledger};
use gent_runtime::{
    Coordinator, RuntimeUpdateAuthority, RuntimeUpdateExecution, RuntimeUpdateExecutionResult,
    RuntimeUpdateExecutor, RuntimeUpdatePlan,
};
use gent_store::SqliteLedger;
use gent_types::{
    CapabilitySet, HostEpoch, RuntimeReleaseArtifact, RuntimeReleaseChannel,
    RuntimeReleaseManifest, RuntimeStagingReceipt, RuntimeUpdateHandoff, RuntimeUpdateRecord,
    RuntimeUpdateStage, RuntimeUpdateStatus, RuntimeVersion, SignedRuntimeRelease,
};

#[derive(Clone, Debug)]
struct Effect {
    calls: Arc<Mutex<u8>>,
    succeeds: bool,
}

impl RuntimeArtifactStager for Effect {
    fn stage(
        &self,
        attempt: &str,
        artifact: &RuntimeReleaseArtifact,
    ) -> Result<RuntimeStagingReceipt, RuntimeUpdatePortError> {
        *self.calls.lock().unwrap() += 1;
        if self.succeeds {
            Ok(RuntimeStagingReceipt {
                attempt_id: attempt.into(),
                artifact_digest_sha256: artifact.digest_sha256.clone(),
            })
        } else {
            Err(RuntimeUpdatePortError::Integrity("staging failed".into()))
        }
    }
}

impl RuntimeHealthProbe for Effect {
    fn probe(&self, _: &RuntimeStagingReceipt) -> Result<(), RuntimeUpdatePortError> {
        *self.calls.lock().unwrap() += 1;
        if self.succeeds {
            Ok(())
        } else {
            Err(RuntimeUpdatePortError::Unavailable("health failed".into()))
        }
    }
}

impl RuntimeActivation for Effect {
    fn activate(&self, _: &RuntimeStagingReceipt) -> Result<(), RuntimeUpdatePortError> {
        *self.calls.lock().unwrap() += 1;
        if self.succeeds {
            Ok(())
        } else {
            Err(RuntimeUpdatePortError::Unavailable(
                "activation failed".into(),
            ))
        }
    }
}

fn plan() -> RuntimeUpdatePlan {
    let artifact = RuntimeReleaseArtifact {
        target: "aarch64-apple-darwin".into(),
        archive_name: "gent.tar.gz".into(),
        digest_sha256: "d".repeat(64),
        size_bytes: 1,
    };
    RuntimeUpdatePlan {
        release: SignedRuntimeRelease {
            key_id: "test".into(),
            signature_hex: "00".into(),
            payload: RuntimeReleaseManifest {
                manifest_version: 1,
                release_version: RuntimeVersion {
                    major: 1,
                    minor: 0,
                    patch: 0,
                },
                protocol_min: 1,
                protocol_max: 1,
                schema_min: 1,
                schema_max: 1,
                minimum_app_version: RuntimeVersion {
                    major: 1,
                    minor: 0,
                    patch: 0,
                },
                channel: RuntimeReleaseChannel::Stable,
                rollout_percent: 100,
                expires_at_unix_seconds: 100,
                revoked: false,
                forward_only_schema: false,
                artifact,
            },
        },
        record: RuntimeUpdateRecord {
            attempt_id: "attempt-1".into(),
            revision: 1,
            artifact_digest_sha256: "d".repeat(64),
            status: RuntimeUpdateStatus {
                stage: RuntimeUpdateStage::Available,
                release_version: None,
                forward_only_schema: false,
                failure: None,
            },
            handoff: RuntimeUpdateHandoff {
                origin_host_epoch: Some(HostEpoch(1)),
                release: Some(gent_types::RuntimeReleaseIdentity {
                    key_id: "test".into(),
                    release_version: RuntimeVersion {
                        major: 1,
                        minor: 0,
                        patch: 0,
                    },
                    target: "aarch64-apple-darwin".into(),
                    artifact_digest_sha256: "d".repeat(64),
                }),
                staging_receipt: None,
            },
        },
    }
}

fn executor(
    authority: RuntimeUpdateAuthority,
    ledger: SqliteLedger,
    stager: Effect,
    health: Effect,
    activation: Effect,
) -> RuntimeUpdateExecutor<SqliteLedger, Effect, Effect, Effect> {
    RuntimeUpdateExecutor::new(
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger,
        stager,
        health,
        activation,
        authority,
    )
}

#[test]
fn observer_executor_cannot_invoke_any_effect() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let calls = Arc::new(Mutex::new(0));
    let effect = Effect {
        calls: calls.clone(),
        succeeds: true,
    };
    let executor = executor(
        RuntimeUpdateAuthority::Observer,
        ledger,
        effect.clone(),
        effect.clone(),
        effect,
    );
    assert_eq!(
        executor
            .execute(RuntimeUpdateExecution::Stage, &plan(), HostEpoch(1), None)
            .unwrap(),
        RuntimeUpdateExecutionResult::DeniedObserver
    );
    assert_eq!(*calls.lock().unwrap(), 0);
}

#[test]
fn approved_executor_stages_health_checks_and_activates_while_ingress_stays_closed() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let plan = plan();
    ledger.save_runtime_update(&plan.record).unwrap();
    let calls = Arc::new(Mutex::new(0));
    let effect = Effect {
        calls: calls.clone(),
        succeeds: true,
    };
    let executor = executor(
        RuntimeUpdateAuthority::Approved,
        ledger.clone(),
        effect.clone(),
        effect.clone(),
        effect,
    );
    let RuntimeUpdateExecutionResult::Staged { receipt, record } = executor
        .execute(RuntimeUpdateExecution::Stage, &plan, HostEpoch(1), None)
        .unwrap()
    else {
        panic!("expected staged")
    };
    assert_eq!(record.status.stage, RuntimeUpdateStage::Staged);
    let RuntimeUpdateExecutionResult::Ready(record) = executor
        .execute(
            RuntimeUpdateExecution::HealthCheck,
            &plan,
            HostEpoch(1),
            Some(&receipt),
        )
        .unwrap()
    else {
        panic!("expected ready")
    };
    assert_eq!(record.status.stage, RuntimeUpdateStage::ReadyToActivate);
    assert_eq!(ledger.host_ingress().unwrap().mode, IngressMode::Closed);
    let RuntimeUpdateExecutionResult::HandoffRequested(record) = executor
        .execute(
            RuntimeUpdateExecution::Activate,
            &plan,
            HostEpoch(1),
            Some(&receipt),
        )
        .unwrap()
    else {
        panic!("expected handoff request")
    };
    assert_eq!(record.status.stage, RuntimeUpdateStage::HandoffRequested);
    assert_eq!(record.handoff.staging_receipt, Some(receipt));
    assert_eq!(*calls.lock().unwrap(), 3);
}

#[test]
fn failed_health_checkpoint_remains_closed_and_is_not_replayed() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let plan = plan();
    ledger.save_runtime_update(&plan.record).unwrap();
    let calls = Arc::new(Mutex::new(0));
    let good = Effect {
        calls: calls.clone(),
        succeeds: true,
    };
    let bad = Effect {
        calls: calls.clone(),
        succeeds: false,
    };
    let executor = executor(
        RuntimeUpdateAuthority::Approved,
        ledger.clone(),
        good.clone(),
        bad,
        good,
    );
    let RuntimeUpdateExecutionResult::Staged { receipt, .. } = executor
        .execute(RuntimeUpdateExecution::Stage, &plan, HostEpoch(1), None)
        .unwrap()
    else {
        panic!("expected staged")
    };
    let RuntimeUpdateExecutionResult::Failed(record) = executor
        .execute(
            RuntimeUpdateExecution::HealthCheck,
            &plan,
            HostEpoch(1),
            Some(&receipt),
        )
        .unwrap()
    else {
        panic!("expected health failure")
    };
    assert_eq!(record.status.stage, RuntimeUpdateStage::Failed);
    assert_eq!(ledger.host_ingress().unwrap().mode, IngressMode::Closed);
    assert!(matches!(
        executor
            .execute(
                RuntimeUpdateExecution::HealthCheck,
                &plan,
                HostEpoch(1),
                Some(&receipt)
            )
            .unwrap(),
        RuntimeUpdateExecutionResult::Existing(_)
    ));
}
