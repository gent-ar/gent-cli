use gent_ports::{PrivateProviderPromptProvisionLedger, ProvisionedProviderLockLedger};

use super::tests;

#[test]
fn mismatched_release_artifact_digest_cannot_settle() {
    let (ledger, saved) = tests::seeded();
    let (binding, command, receipt) = tests::provision(&ledger, &saved);
    let mut mismatched = tests::installation(&command, &binding);
    mismatched.provenance.release_artifact_digest_sha256 = "f".repeat(64);
    assert!(
        ledger
            .settle_verified_provider_prompt_provision(
                &command,
                &receipt,
                &mismatched,
                &tests::terminal(&receipt),
                &binding,
            )
            .is_err()
    );
    assert_eq!(
        tests::dispatch_state(&ledger, &saved.message.message_id),
        "provisioning"
    );
    assert!(
        ledger
            .find_provisioned_provider_installation("codex")
            .unwrap()
            .is_none()
    );
}

#[test]
fn mismatched_receipt_fingerprint_cannot_settle() {
    let (ledger, saved) = tests::seeded();
    let (binding, command, receipt) = tests::provision(&ledger, &saved);
    let mut mismatched = tests::installation(&command, &binding);
    mismatched.provenance.receipt_fingerprint_sha256 = "f".repeat(64);
    assert!(
        ledger
            .settle_verified_provider_prompt_provision(
                &command,
                &receipt,
                &mismatched,
                &tests::terminal(&receipt),
                &binding,
            )
            .is_err()
    );
    assert_eq!(
        tests::dispatch_state(&ledger, &saved.message.message_id),
        "provisioning"
    );
    assert!(
        ledger
            .find_provisioned_provider_installation("codex")
            .unwrap()
            .is_none()
    );
}
