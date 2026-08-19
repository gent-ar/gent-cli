use gent_ports::{PrivateProviderPromptProvisionLedger, ProvisionedProviderLockLedger};

use super::tests;

#[test]
fn package_provenance_mismatch_rolls_back_lock_receipt_and_prompt_release() {
    let (ledger, saved) = tests::seeded();
    let (binding, command, receipt) = tests::provision(&ledger, &saved);
    let mut wrong_installation = tests::installation();
    wrong_installation.provenance.package_version = "2.0.0".into();
    assert!(
        ledger
            .settle_verified_provider_prompt_provision(
                &command,
                &receipt,
                &wrong_installation,
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
    assert_eq!(
        tests::receipt_status(&ledger, &receipt.idempotency_key),
        "accepted"
    );
}
