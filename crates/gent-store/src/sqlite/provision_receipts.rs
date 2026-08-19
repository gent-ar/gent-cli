//! Exact durable receipt reads for uncomposed private provisioning.

use gent_ports::LedgerError;
use gent_types::{Command, Receipt};
use rusqlite::{OptionalExtension, params};

use super::{SqliteLedger, queries::decode_status, queries::storage_error};

impl SqliteLedger {
    /// Reads a receipt only when its complete durable command identity still matches.
    ///
    /// # Errors
    /// Returns an error when command fingerprinting or the durable ledger read fails.
    pub fn find_command_receipt_binding(
        &self,
        command: &Command,
    ) -> Result<Option<Receipt>, LedgerError> {
        let fingerprint = super::queries::command_fingerprint(command)?;
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT status FROM receipts WHERE receipt_id = ?1 AND idempotency_key = ?2 AND host_epoch = ?3 AND kind = ?4 AND payload_digest = ?5",
                params![&command.receipt_id.0, &command.idempotency_key, command.host_epoch.0, &command.kind, fingerprint],
                |row| Ok(Receipt {
                    receipt_id: command.receipt_id.clone(),
                    idempotency_key: command.idempotency_key.clone(),
                    status: decode_status(&row.get::<_, String>(0)?)?,
                    host_epoch: command.host_epoch,
                }),
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
    fn command_binding_rejects_a_different_provider_operation() {
        let ledger = SqliteLedger::in_memory().unwrap();
        let receipt_id = ReceiptId("receipt-1".into());
        let accepted_command = command(receipt_id.clone(), "codex");
        ledger
            .claim_command(
                &accepted_command,
                &Event {
                    cursor: 0,
                    event_id: "accepted-1".into(),
                    receipt_id: receipt_id.clone(),
                    host_epoch: HostEpoch(1),
                    kind: "accepted".into(),
                    payload: accepted_command.payload.clone(),
                },
            )
            .unwrap();
        assert!(
            ledger
                .find_command_receipt_binding(&accepted_command)
                .unwrap()
                .is_some()
        );
        assert!(
            ledger
                .find_command_receipt_binding(&command(receipt_id, "claude"))
                .unwrap()
                .is_none()
        );
    }

    fn command(receipt_id: ReceiptId, provider: &str) -> Command {
        Command {
            receipt_id,
            idempotency_key: "key-1".into(),
            host_epoch: HostEpoch(1),
            kind: "dependencyAction".into(),
            payload: json!({
                "action": "install",
                "consentGranted": true,
                "provider": provider,
                "reviewedPlanDigest": "digest",
            }),
        }
    }
}
