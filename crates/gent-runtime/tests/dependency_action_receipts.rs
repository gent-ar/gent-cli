use gent_ports::{Ledger, ReceiptClaim};
use gent_protocol::{DependencyAction, DependencyActionRequest, DependencyProvider};
use gent_runtime::{
    DependencyActionReceiptClaim, DependencyActionReceiptReservation, dependency_action_command,
};
use gent_store::SqliteLedger;
use gent_types::{Event, HostEpoch, ReceiptId, ReceiptStatus};

#[test]
fn first_exact_request_claims_one_accepted_receipt() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let reservation = DependencyActionReceiptReservation::new(ledger.clone());
    let request = request("first");

    let receipt = match reservation.reserve(&request).unwrap() {
        DependencyActionReceiptClaim::Claimed(receipt) => receipt,
        other => panic!("expected first receipt claim, got {other:?}"),
    };

    assert_eq!(receipt.receipt_id, request.receipt_id);
    assert_eq!(receipt.idempotency_key, request.idempotency_key);
    assert_eq!(receipt.status, ReceiptStatus::Accepted);
    assert_eq!(receipt.host_epoch, request.host_epoch);
}

#[test]
fn preexisting_accepted_receipt_requires_unprovable_recovery_not_replay() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let request = request("accepted-recovery");
    let command = dependency_action_command(&request);
    let accepted = claim(&ledger, &command);
    let reservation = DependencyActionReceiptReservation::new(ledger);

    assert_eq!(
        reservation.reserve(&request).unwrap(),
        DependencyActionReceiptClaim::AcceptedRecovery(accepted)
    );
}

#[test]
fn terminal_receipt_is_returned_without_a_new_claim() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let request = request("terminal");
    let command = dependency_action_command(&request);
    let accepted = claim(&ledger, &command);
    let terminal = Event {
        cursor: 0,
        event_id: "terminal-event".into(),
        receipt_id: accepted.receipt_id.clone(),
        host_epoch: accepted.host_epoch,
        kind: "dependencyActionCompleted".into(),
        payload: serde_json::json!({ "status": ReceiptStatus::Settled }),
    };
    let settled = ledger
        .settle_receipt(&accepted.idempotency_key, ReceiptStatus::Settled, &terminal)
        .unwrap();
    let reservation = DependencyActionReceiptReservation::new(ledger);

    assert_eq!(
        reservation.reserve(&request).unwrap(),
        DependencyActionReceiptClaim::Terminal(settled)
    );
}

fn request(key: &str) -> DependencyActionRequest {
    DependencyActionRequest {
        provider: DependencyProvider::Codex,
        action: DependencyAction::Install,
        consent_granted: true,
        receipt_id: ReceiptId(format!("receipt-{key}")),
        idempotency_key: key.into(),
        host_epoch: HostEpoch(1),
        reviewed_plan_digest: "reviewed-plan".into(),
    }
}

fn claim(ledger: &SqliteLedger, command: &gent_types::Command) -> gent_types::Receipt {
    match ledger
        .claim_command(
            command,
            &Event {
                cursor: 0,
                event_id: format!("{}:accepted", command.receipt_id.0),
                receipt_id: command.receipt_id.clone(),
                host_epoch: command.host_epoch,
                kind: "dependencyActionAccepted".into(),
                payload: command.payload.clone(),
            },
        )
        .unwrap()
    {
        ReceiptClaim::Accepted(receipt) => receipt,
        ReceiptClaim::Existing(_) => panic!("fixture must claim a fresh receipt"),
    }
}
