//! Receipt decoding local to atomic agent-chat conversation creation.

use gent_ports::LedgerError;
use gent_types::ReceiptStatus;

pub(super) fn decode_create_receipt_status(value: &str) -> Result<ReceiptStatus, LedgerError> {
    match value {
        "settled" => Ok(ReceiptStatus::Settled),
        "accepted" | "unprovable" | "rejected" => Err(LedgerError::Invariant(
            "agent chat create receipt has an invalid terminal state".into(),
        )),
        _ => Err(LedgerError::Storage("unknown receipt status".into())),
    }
}
