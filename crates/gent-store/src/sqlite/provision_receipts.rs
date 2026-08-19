//! Exact durable receipt reads for uncomposed private provisioning.

use gent_ports::LedgerError;
use gent_types::{HostEpoch, Receipt, ReceiptId};
use rusqlite::{OptionalExtension, params};

use super::{SqliteLedger, queries::decode_status, queries::storage_error};

impl SqliteLedger {
    /// Reads a receipt only when all provisioning authority fences still match.
    ///
    /// This narrow DAO deliberately does not claim, settle, or mutate a receipt. A future
    /// private provisioner uses it immediately before its one external npm effect.
    ///
    /// # Errors
    /// Returns an error when the durable ledger cannot be read.
    pub fn find_receipt_binding(
        &self,
        receipt_id: &ReceiptId,
        idempotency_key: &str,
        host_epoch: HostEpoch,
    ) -> Result<Option<Receipt>, LedgerError> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT status FROM receipts WHERE receipt_id = ?1 AND idempotency_key = ?2 AND host_epoch = ?3",
                params![&receipt_id.0, idempotency_key, host_epoch.0],
                |row| {
                    Ok(Receipt {
                        receipt_id: receipt_id.clone(),
                        idempotency_key: idempotency_key.into(),
                        status: decode_status(&row.get::<_, String>(0)?)?,
                        host_epoch,
                    })
                },
            )
            .optional()
            .map_err(storage_error)
    }
}

#[cfg(test)]
mod tests {
    use gent_ports::Ledger;
    use gent_types::{Command, Event, HostEpoch, ReceiptId};
    use serde_json::json;

    use super::SqliteLedger;

    #[test]
    fn receipt_binding_requires_every_authority_identity() {
        let ledger = SqliteLedger::in_memory().unwrap();
        let receipt_id = ReceiptId("receipt-1".into());
        ledger
            .claim_command(
                &Command {
                    receipt_id: receipt_id.clone(),
                    idempotency_key: "key-1".into(),
                    host_epoch: HostEpoch(1),
                    kind: "provision".into(),
                    payload: json!({}),
                },
                &Event {
                    cursor: 0,
                    event_id: "accepted-1".into(),
                    receipt_id: receipt_id.clone(),
                    host_epoch: HostEpoch(1),
                    kind: "accepted".into(),
                    payload: json!({}),
                },
            )
            .unwrap();
        assert!(
            ledger
                .find_receipt_binding(&receipt_id, "key-1", HostEpoch(1))
                .unwrap()
                .is_some()
        );
        for (id, key, epoch) in [
            (ReceiptId("other".into()), "key-1", HostEpoch(1)),
            (receipt_id.clone(), "other", HostEpoch(1)),
            (receipt_id, "key-1", HostEpoch(2)),
        ] {
            assert!(
                ledger
                    .find_receipt_binding(&id, key, epoch)
                    .unwrap()
                    .is_none()
            );
        }
    }
}
