use serde_json::json;

use crate::{Command, HostEpoch, ReceiptId};

fn command() -> Command {
    Command {
        receipt_id: ReceiptId("receipt-1".into()),
        idempotency_key: "idempotency-1".into(),
        host_epoch: HostEpoch(7),
        kind: "example.command".into(),
        payload: json!({"a": 1}),
    }
}

#[test]
fn receipt_fingerprint_is_stable_for_an_identical_command() {
    assert_eq!(
        command().receipt_fingerprint_sha256(),
        command().receipt_fingerprint_sha256()
    );
}

#[test]
fn receipt_fingerprint_changes_with_every_correlation_or_content_field() {
    let base = command().receipt_fingerprint_sha256();
    assert_ne!(
        base,
        Command {
            receipt_id: ReceiptId("receipt-2".into()),
            ..command()
        }
        .receipt_fingerprint_sha256()
    );
    assert_ne!(
        base,
        Command {
            idempotency_key: "idempotency-2".into(),
            ..command()
        }
        .receipt_fingerprint_sha256()
    );
    assert_ne!(
        base,
        Command {
            host_epoch: HostEpoch(8),
            ..command()
        }
        .receipt_fingerprint_sha256()
    );
    assert_ne!(
        base,
        Command {
            kind: "other.command".into(),
            ..command()
        }
        .receipt_fingerprint_sha256()
    );
    assert_ne!(
        base,
        Command {
            payload: json!({"a": 2}),
            ..command()
        }
        .receipt_fingerprint_sha256()
    );
}
