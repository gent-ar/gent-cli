use super::*;
use gent_ports::{PackageInstallPolicyError, ProvisionedProviderLockLedger};
use gent_protocol::DependencyProvider;
use gent_store::SqliteLedger;
use gent_types::{
    HostEpoch, ProviderInstallProvenance, ProvisionedProviderInstallation, ProvisionedProviderLock,
    ReceiptId, RunVersionLock,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[derive(Clone)]
struct Policy(Arc<Mutex<ApprovedPackageInstall>>);
impl PackageInstallPolicy for Policy {
    fn approved_package(
        &self,
        _: &str,
        _: u64,
    ) -> Result<ApprovedPackageInstall, PackageInstallPolicyError> {
        Ok(self.0.lock().unwrap().clone())
    }
}
#[derive(Clone)]
struct Clock;
impl AuthorityClock for Clock {
    fn now_unix_seconds(&self) -> u64 {
        10
    }
}
type Barrier = (
    std::sync::mpsc::SyncSender<()>,
    std::sync::mpsc::Receiver<()>,
);
#[derive(Clone, Default)]
struct Effect {
    calls: Arc<AtomicUsize>,
    ambiguous: Arc<AtomicBool>,
    fence: Arc<Mutex<Option<(SqliteLedger, bool)>>>,
    barrier: Arc<Mutex<Option<Barrier>>>,
}
impl ManagedProviderEffect for Effect {
    fn provision_reviewed(
        &self,
        request: &PrivateProvisionRequest,
        command: &gent_types::Command,
        review: &ReviewedPackage,
    ) -> Result<PrivateProvisionResult, crate::private_provider_provisioning::PrivateProvisionError>
    {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some((entered, resume)) = self.barrier.lock().unwrap().take() {
            entered.send(()).unwrap();
            resume
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        }
        if let Some((ledger, closed)) = self.fence.lock().unwrap().take() {
            ledger.close_ingress(HostEpoch(1)).unwrap();
            if !closed {
                ledger.fence_and_open(HostEpoch(1)).unwrap();
            }
        }
        if self.ambiguous.load(Ordering::SeqCst) {
            return Ok(PrivateProvisionResult::Ambiguous);
        }
        assert_eq!(request.action, DependencyAction::Update);
        Ok(PrivateProvisionResult::Installed(Box::new(
            ProvisionedProviderInstallation {
                lock: ProvisionedProviderLock {
                    run_lock: RunVersionLock {
                        provider: request.provider.as_str().into(),
                        canonical_path: format!(
                            "/owned/generations/{}/codex",
                            command.receipt_fingerprint_sha256()
                        ),
                        file_identity: "file".into(),
                        digest_sha256: "a".repeat(64),
                        version: review.package.version.clone(),
                        compatibility_entry: "approved".into(),
                    },
                },
                provenance: ProviderInstallProvenance {
                    package_name: review.package.package_name.clone(),
                    package_version: review.package.version.clone(),
                    package_integrity: review.package.integrity.clone(),
                    package_policy_digest_sha256: review
                        .package
                        .package_policy_digest_sha256
                        .clone(),
                    node_runtime_digest_sha256: "c".repeat(64),
                    release_artifact_digest_sha256: review.release_digest.clone(),
                    receipt_fingerprint_sha256: command.receipt_fingerprint_sha256(),
                },
            },
        )))
    }
}
type Boundary = ManagedProviderActions<SqliteLedger, Policy, Effect, Clock>;
fn fixture() -> (Boundary, Effect, Policy) {
    let policy = Policy(Arc::new(Mutex::new(ApprovedPackageInstall {
        provider: "codex".into(),
        package_name: "@openai/codex".into(),
        version: "0.153.4-win32-x64".into(),
        integrity: "sha512-approved".into(),
        package_policy_digest_sha256: "b".repeat(64),
    })));
    let effect = Effect::default();
    (
        Boundary::new(
            SqliteLedger::in_memory().unwrap(),
            policy.clone(),
            effect.clone(),
            Clock,
            "d".repeat(64),
        ),
        effect,
        policy,
    )
}
fn request(boundary: &Boundary, key: &str) -> DependencyActionRequest {
    let plan = boundary.plan(DependencyPlanRequest {
        provider: DependencyProvider::Codex,
        action: DependencyAction::Update,
    });
    DependencyActionRequest {
        provider: DependencyProvider::Codex,
        action: DependencyAction::Update,
        consent_granted: true,
        receipt_id: ReceiptId(format!("receipt-{key}")),
        idempotency_key: key.into(),
        host_epoch: HostEpoch(1),
        reviewed_plan_digest: plan.reviewed_plan_digest,
    }
}
#[test]
fn explicit_same_version_update_publishes_a_new_receipt_bound_generation_once() {
    let (b, e, _) = fixture();
    let req = request(&b, "one");
    let first = b.execute(req.clone()).unwrap();
    assert_eq!(first.state, DependencyActionState::Completed);
    let old = b
        .ledger
        .find_provisioned_provider_installation("codex")
        .unwrap()
        .unwrap();
    assert_eq!(
        old.provenance.receipt_fingerprint_sha256,
        gent_runtime::dependency_action_command(&req).receipt_fingerprint_sha256()
    );
    assert_eq!(
        b.execute(req).unwrap().state,
        DependencyActionState::Completed
    );
    assert_eq!(e.calls.load(Ordering::SeqCst), 1);
    b.execute(request(&b, "two")).unwrap();
    let next = b
        .ledger
        .find_provisioned_provider_installation("codex")
        .unwrap()
        .unwrap();
    assert_ne!(
        old.lock.run_lock.canonical_path,
        next.lock.run_lock.canonical_path
    );
    assert_eq!(e.calls.load(Ordering::SeqCst), 2);
}
#[test]
fn denied_consent_tampered_plan_and_stale_epoch_have_no_effect() {
    let (b, e, _) = fixture();
    let mut req = request(&b, "no-consent");
    req.consent_granted = false;
    assert_eq!(
        b.execute(req).unwrap().state,
        DependencyActionState::ConsentRequired
    );
    let mut req = request(&b, "tampered");
    req.reviewed_plan_digest = "e".repeat(64);
    assert_eq!(
        b.execute(req).unwrap().state,
        DependencyActionState::PlanMismatch
    );
    let mut req = request(&b, "stale");
    req.host_epoch = HostEpoch(2);
    assert!(b.execute(req).is_err());
    assert_eq!(e.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn changed_signed_package_invalidates_review_before_effect() {
    let (b, e, p) = fixture();
    let req = request(&b, "old-review");
    p.0.lock().unwrap().integrity = "sha512-new".into();
    assert_eq!(
        b.execute(req).unwrap().state,
        DependencyActionState::PlanMismatch
    );
    assert_eq!(e.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn accepted_recovery_is_unprovable_without_replay() {
    let (b, e, _) = fixture();
    let req = request(&b, "recovery");
    DependencyActionReceiptReservation::new(b.ledger.clone())
        .reserve(&req)
        .unwrap();
    assert_eq!(
        b.execute(req.clone()).unwrap().state,
        DependencyActionState::Unprovable
    );
    assert_eq!(
        b.execute(req).unwrap().state,
        DependencyActionState::Unprovable
    );
    assert_eq!(e.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn ambiguous_update_preserves_previous_installed_lock() {
    let (b, e, _) = fixture();
    b.execute(request(&b, "old")).unwrap();
    let old = b
        .ledger
        .find_provisioned_provider_installation("codex")
        .unwrap();
    e.ambiguous.store(true, Ordering::SeqCst);
    let req = request(&b, "uncertain");
    assert_eq!(
        b.execute(req.clone()).unwrap().state,
        DependencyActionState::Unprovable
    );
    assert_eq!(
        b.execute(req).unwrap().state,
        DependencyActionState::Unprovable
    );
    assert_eq!(
        b.ledger
            .find_provisioned_provider_installation("codex")
            .unwrap(),
        old
    );
    assert_eq!(e.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn host_fence_during_effect_prevents_atomic_publication() {
    for closed in [false, true] {
        let (b, e, _) = fixture();
        b.execute(request(&b, "previous")).unwrap();
        let old = b
            .ledger
            .find_provisioned_provider_installation("codex")
            .unwrap();
        *e.fence.lock().unwrap() = Some((b.ledger.clone(), closed));
        let req = request(&b, "fenced-update");
        let command = gent_runtime::dependency_action_command(&req);
        assert!(b.execute(req).is_err());
        assert_eq!(
            b.ledger
                .find_provisioned_provider_installation("codex")
                .unwrap(),
            old
        );
        assert_eq!(
            b.ledger
                .find_command_receipt_binding(&command)
                .unwrap()
                .unwrap()
                .status,
            ReceiptStatus::Accepted
        );
    }
}

#[path = "lifecycle_tests.rs"]
mod lifecycle_tests;
