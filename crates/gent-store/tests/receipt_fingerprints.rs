use gent_ports::{Ledger, ReceiptClaim};
use gent_store::SqliteLedger;
use gent_types::{Command, Event, HostEpoch, ReceiptId};

fn command(receipt_id: &str, payload: serde_json::Value) -> Command {
    Command {
        receipt_id: ReceiptId(receipt_id.into()),
        idempotency_key: "once".into(),
        host_epoch: HostEpoch(1),
        kind: "attachmentAppend".into(),
        payload,
    }
}

fn accepted(command: &Command) -> Event {
    Event {
        cursor: 0,
        event_id: format!("{}:accepted", command.receipt_id.0),
        receipt_id: command.receipt_id.clone(),
        host_epoch: command.host_epoch,
        kind: "attachmentAccepted".into(),
        payload: serde_json::json!({}),
    }
}

#[test]
fn idempotency_retries_must_preserve_receipt_epoch_kind_and_payload() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let first = command(
        "receipt-1",
        serde_json::json!({ "offset": 0, "digest": "a" }),
    );
    assert!(matches!(
        ledger.claim_command(&first, &accepted(&first)).unwrap(),
        ReceiptClaim::Accepted(_)
    ));
    assert!(matches!(
        ledger.claim_command(&first, &accepted(&first)).unwrap(),
        ReceiptClaim::Existing(_)
    ));
    let changed_payload = command(
        "receipt-1",
        serde_json::json!({ "offset": 0, "digest": "b" }),
    );
    assert!(
        ledger
            .claim_command(&changed_payload, &accepted(&changed_payload))
            .is_err()
    );
    let changed_receipt = command("receipt-2", first.payload.clone());
    assert!(
        ledger
            .claim_command(&changed_receipt, &accepted(&changed_receipt))
            .is_err()
    );
}
