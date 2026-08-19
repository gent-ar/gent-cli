use gent_ports::{Ledger, ProvisionedProviderLockLedger, ReceiptClaim};
use gent_protocol::{DependencyAction, DependencyActionRequest, DependencyProvider};
use gent_store::SqliteLedger;
use gent_types::{
    Event, HostEpoch, ProviderInstallProvenance, ProvisionedProviderInstallation,
    ProvisionedProviderLock, ReceiptId, ReceiptStatus, RunVersionLock,
};
use serde_json::json;

use super::{PrivateProvisionSettlementCoordinator, PrivateProvisionSettlementState};
use crate::private_provider_provisioning::{PrivateProvisionRequest, PrivateProvisionResult};

#[test]
fn verified_installation_is_persisted_with_the_same_settlement() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let request = request(&ledger, "installed");
    let settlement = PrivateProvisionSettlementCoordinator::new(ledger.clone())
        .settle(
            &request,
            PrivateProvisionResult::Installed(Box::new(installation())),
        )
        .unwrap();
    assert_eq!(settlement.state, PrivateProvisionSettlementState::Installed);
    assert_eq!(settlement.receipt.status, ReceiptStatus::Settled);
    assert_eq!(
        ledger
            .find_provisioned_provider_installation("codex")
            .unwrap(),
        Some(installation())
    );
}

#[test]
fn ambiguous_result_is_unprovable_and_cannot_be_replayed() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let request = request(&ledger, "ambiguous");
    let coordinator = PrivateProvisionSettlementCoordinator::new(ledger.clone());
    let first = coordinator
        .settle(&request, PrivateProvisionResult::Ambiguous)
        .unwrap();
    assert_eq!(first.state, PrivateProvisionSettlementState::Unprovable);
    assert_eq!(first.receipt.status, ReceiptStatus::Unprovable);
    assert_eq!(
        coordinator
            .settle(&request, PrivateProvisionResult::Ambiguous)
            .unwrap()
            .receipt
            .status,
        ReceiptStatus::Unprovable
    );
    assert!(
        ledger
            .find_provisioned_provider_installation("codex")
            .unwrap()
            .is_none()
    );
}

#[test]
fn consent_required_is_rejected_and_exact_receipt_mismatch_fails_closed() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let request = request(&ledger, "consent");
    let coordinator = PrivateProvisionSettlementCoordinator::new(ledger);
    assert_eq!(
        coordinator
            .settle(&request, PrivateProvisionResult::ConsentRequired)
            .unwrap()
            .state,
        PrivateProvisionSettlementState::ConsentRequired
    );
    let mut mismatched = request;
    mismatched.receipt.host_epoch = HostEpoch(2);
    assert!(
        coordinator
            .settle(&mismatched, PrivateProvisionResult::Ambiguous)
            .is_err()
    );
}

fn request(ledger: &SqliteLedger, key: &str) -> PrivateProvisionRequest {
    let receipt_id = ReceiptId(format!("receipt-{key}"));
    let command = gent_runtime::dependency_action_command(&DependencyActionRequest {
        receipt_id: receipt_id.clone(),
        idempotency_key: key.into(),
        host_epoch: HostEpoch(1),
        provider: DependencyProvider::Codex,
        action: DependencyAction::Install,
        consent_granted: true,
        reviewed_plan_digest: "digest".into(),
    });
    let receipt = match ledger
        .claim_command(
            &command,
            &Event {
                cursor: 0,
                event_id: format!("accepted-{key}"),
                receipt_id,
                host_epoch: HostEpoch(1),
                kind: "accepted".into(),
                payload: json!({}),
            },
        )
        .unwrap()
    {
        ReceiptClaim::Accepted(receipt) => receipt,
        ReceiptClaim::Existing(_) => panic!("receipt must be new"),
    };
    PrivateProvisionRequest {
        receipt,
        provider: DependencyProvider::Codex,
        action: DependencyAction::Install,
        reviewed_plan_digest: "digest".into(),
        consent_granted: true,
        now_unix_seconds: 0,
    }
}

fn installation() -> ProvisionedProviderInstallation {
    ProvisionedProviderInstallation {
        lock: ProvisionedProviderLock {
            run_lock: RunVersionLock {
                provider: "codex".into(),
                canonical_path: "/private/codex".into(),
                file_identity: "identity".into(),
                digest_sha256: "a".repeat(64),
                version: "1.0.0".into(),
                compatibility_entry: "entry".into(),
            },
        },
        provenance: ProviderInstallProvenance {
            package_name: "@openai/codex".into(),
            package_version: "1.0.0".into(),
            package_integrity: "sha512-test".into(),
            package_policy_digest_sha256: "b".repeat(64),
            node_runtime_digest_sha256: "c".repeat(64),
        },
    }
}
