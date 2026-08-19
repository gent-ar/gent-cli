use gent_ports::{Ledger, ProvisionedProviderLockLedger, ReceiptClaim};
use gent_types::{
    Command, Event, HostEpoch, ProviderInstallProvenance, ProvisionedProviderInstallation,
    ProvisionedProviderLock, Receipt, ReceiptId, ReceiptStatus, RunVersionLock,
};
use serde_json::json;

use super::SqliteLedger;

#[test]
fn installation_and_settlement_are_one_atomic_commit() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let receipt = accepted(&ledger, "one");
    let mut conflicting = terminal(&receipt);
    conflicting.event_id = "accepted-one".into();
    assert!(
        ledger
            .settle_provisioned_provider_provision(
                &receipt,
                Some(&installation("v1")),
                ReceiptStatus::Settled,
                &conflicting,
            )
            .is_err()
    );
    assert!(
        ledger
            .find_provisioned_provider_installation("codex")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        settle(&ledger, &receipt, Some(&installation("v1"))),
        ReceiptStatus::Settled
    );
}

#[test]
fn settled_locks_are_immutable_and_latest_provider_lock_wins() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let first = accepted(&ledger, "one");
    let second = accepted(&ledger, "two");
    settle(&ledger, &first, Some(&installation("v1")));
    settle(&ledger, &second, Some(&installation("v2")));
    assert_eq!(
        ledger
            .find_provisioned_provider_installation("codex")
            .unwrap(),
        Some(installation("v2"))
    );
    assert!(
        ledger
            .settle_provisioned_provider_provision(
                &first,
                Some(&installation("changed")),
                ReceiptStatus::Settled,
                &terminal(&first),
            )
            .is_err()
    );
}

#[test]
fn terminal_non_installation_cannot_create_a_lock() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let receipt = accepted(&ledger, "one");
    assert_eq!(settle(&ledger, &receipt, None), ReceiptStatus::Unprovable);
    assert!(
        ledger
            .settle_provisioned_provider_provision(
                &receipt,
                Some(&installation("v1")),
                ReceiptStatus::Settled,
                &terminal(&receipt),
            )
            .is_err()
    );
    assert!(
        ledger
            .find_provisioned_provider_installation("codex")
            .unwrap()
            .is_none()
    );
}

#[test]
fn malformed_provenance_rolls_back_without_settling() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let receipt = accepted(&ledger, "one");
    let mut invalid = installation("v1");
    invalid.provenance.node_runtime_digest_sha256 = "not-a-digest".into();
    assert!(
        ledger
            .settle_provisioned_provider_provision(
                &receipt,
                Some(&invalid),
                ReceiptStatus::Settled,
                &terminal(&receipt),
            )
            .is_err()
    );
    assert_eq!(settle(&ledger, &receipt, None), ReceiptStatus::Unprovable);
}

fn settle(
    ledger: &SqliteLedger,
    receipt: &Receipt,
    installation: Option<&ProvisionedProviderInstallation>,
) -> ReceiptStatus {
    let status = if installation.is_some() {
        ReceiptStatus::Settled
    } else {
        ReceiptStatus::Unprovable
    };
    ledger
        .settle_provisioned_provider_provision(
            receipt,
            installation,
            status.clone(),
            &terminal(receipt),
        )
        .unwrap()
        .status
}

fn accepted(ledger: &SqliteLedger, key: &str) -> Receipt {
    let receipt_id = ReceiptId(format!("receipt-{key}"));
    let command = Command {
        receipt_id: receipt_id.clone(),
        idempotency_key: key.into(),
        host_epoch: HostEpoch(1),
        kind: "dependencyAction".into(),
        payload: json!({ "provider": "codex" }),
    };
    match ledger
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
    }
}

fn terminal(receipt: &Receipt) -> Event {
    Event {
        cursor: 0,
        event_id: format!("terminal-{}", receipt.idempotency_key),
        receipt_id: receipt.receipt_id.clone(),
        host_epoch: receipt.host_epoch,
        kind: "terminal".into(),
        payload: json!({}),
    }
}

fn installation(version: &str) -> ProvisionedProviderInstallation {
    ProvisionedProviderInstallation {
        lock: ProvisionedProviderLock {
            run_lock: RunVersionLock {
                provider: "codex".into(),
                canonical_path: format!("/private/{version}/codex"),
                file_identity: format!("identity-{version}"),
                digest_sha256: "a".repeat(64),
                version: version.into(),
                compatibility_entry: format!("entry-{version}"),
            },
        },
        provenance: ProviderInstallProvenance {
            package_name: "@openai/codex".into(),
            package_version: version.into(),
            package_integrity: "sha512-test".into(),
            package_policy_digest_sha256: "b".repeat(64),
            node_runtime_digest_sha256: "c".repeat(64),
        },
    }
}
