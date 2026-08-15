//! Shared durable receipt transitions for attachment mutations.

use gent_ports::{Ledger, ReceiptClaim};
use gent_types::{Command, Event, HostEpoch, ReceiptId, ReceiptStatus};

use crate::RuntimeError;

pub(super) fn claim<L: Ledger>(
    ledger: &L,
    receipt_id: &ReceiptId,
    idempotency_key: &str,
    host_epoch: HostEpoch,
    kind: &str,
    payload: serde_json::Value,
) -> Result<bool, RuntimeError> {
    let command = Command {
        receipt_id: receipt_id.clone(),
        idempotency_key: idempotency_key.into(),
        host_epoch,
        kind: kind.into(),
        payload,
    };
    let accepted = Event {
        cursor: 0,
        event_id: format!("{}:accepted", receipt_id.0),
        receipt_id: receipt_id.clone(),
        host_epoch,
        kind: "attachmentAccepted".into(),
        payload: serde_json::json!({ "operation": kind }),
    };
    match ledger.claim_command(&command, &accepted)? {
        ReceiptClaim::Accepted(_)
        | ReceiptClaim::Existing(gent_types::Receipt {
            status: ReceiptStatus::Accepted,
            ..
        }) => Ok(true),
        ReceiptClaim::Existing(_) => Ok(false),
    }
}

pub(super) fn settle<L: Ledger>(
    ledger: &L,
    receipt_id: &ReceiptId,
    idempotency_key: &str,
    host_epoch: HostEpoch,
    kind: &str,
    result: Result<gent_types::AttachmentTransfer, RuntimeError>,
) -> Result<gent_types::AttachmentTransfer, RuntimeError> {
    let (status, result) = match result {
        Ok(transfer) => (ReceiptStatus::Settled, Ok(transfer)),
        Err(error) => (ReceiptStatus::Rejected, Err(error)),
    };
    let terminal = Event {
        cursor: 0,
        event_id: format!("{}:terminal", receipt_id.0),
        receipt_id: receipt_id.clone(),
        host_epoch,
        kind: "attachmentSettled".into(),
        payload: serde_json::json!({ "operation": kind, "status": status }),
    };
    ledger.settle_receipt(idempotency_key, status, &terminal)?;
    result
}
