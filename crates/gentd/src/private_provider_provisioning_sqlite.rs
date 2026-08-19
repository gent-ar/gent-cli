//! `SQLite` receipt adapter for the dormant Gent-only provisioning seam.
//!
//! Bootstrap never constructs this adapter. It carries no policy, installer, or provider runner;
//! it merely re-reads an exact receipt binding immediately before a future approved effect.

use gent_store::SqliteLedger;
use gent_types::{HostEpoch, Receipt, ReceiptId, ReceiptStatus};

use crate::private_provider_provisioning::ProvisionReceiptReader;

/// Reads the one exact accepted receipt from Gent's own `SQLite` ledger.
#[derive(Clone, Debug)]
pub(crate) struct SqliteProvisionReceiptReader {
    ledger: SqliteLedger,
}

impl SqliteProvisionReceiptReader {
    /// Binds the daemon-owned Gent ledger without enabling any provisioning authority.
    #[must_use]
    pub(crate) const fn new(ledger: SqliteLedger) -> Self {
        Self { ledger }
    }
}

impl ProvisionReceiptReader for SqliteProvisionReceiptReader {
    fn accepted_receipt(
        &self,
        receipt_id: &ReceiptId,
        idempotency_key: &str,
        host_epoch: HostEpoch,
    ) -> Result<Receipt, String> {
        let receipt = self
            .ledger
            .find_receipt_binding(receipt_id, idempotency_key, host_epoch)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "provisioning receipt binding is missing".to_owned())?;
        (receipt.status == ReceiptStatus::Accepted)
            .then_some(receipt)
            .ok_or_else(|| "provisioning receipt is not accepted".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use gent_ports::{Ledger, ReceiptClaim};
    use gent_store::SqliteLedger;
    use gent_types::{Command, Event, HostEpoch, ReceiptId, ReceiptStatus};
    use serde_json::json;

    use super::{ProvisionReceiptReader, SqliteProvisionReceiptReader};

    #[test]
    fn exact_accepted_receipt_is_read_from_gent_sqlite() {
        let ledger = SqliteLedger::in_memory().unwrap();
        let receipt_id = ReceiptId("receipt-1".into());
        let accepted = accepted(&ledger, receipt_id.clone(), "install-1", HostEpoch(1));
        let reader = SqliteProvisionReceiptReader::new(ledger);
        assert_eq!(
            reader
                .accepted_receipt(&receipt_id, "install-1", HostEpoch(1))
                .unwrap(),
            accepted
        );
    }

    #[test]
    fn mismatched_or_terminal_receipts_are_refused() {
        let ledger = SqliteLedger::in_memory().unwrap();
        let receipt_id = ReceiptId("receipt-1".into());
        accepted(&ledger, receipt_id.clone(), "install-1", HostEpoch(1));
        ledger
            .settle_receipt(
                "install-1",
                ReceiptStatus::Unprovable,
                &Event {
                    cursor: 0,
                    event_id: "terminal-1".into(),
                    receipt_id: receipt_id.clone(),
                    host_epoch: HostEpoch(1),
                    kind: "terminal".into(),
                    payload: json!({}),
                },
            )
            .unwrap();
        let reader = SqliteProvisionReceiptReader::new(ledger);
        assert!(
            reader
                .accepted_receipt(&receipt_id, "install-1", HostEpoch(1))
                .is_err()
        );
        assert!(
            reader
                .accepted_receipt(&receipt_id, "other", HostEpoch(1))
                .is_err()
        );
    }

    fn accepted(
        ledger: &SqliteLedger,
        receipt_id: ReceiptId,
        idempotency_key: &str,
        host_epoch: HostEpoch,
    ) -> gent_types::Receipt {
        match ledger
            .claim_command(
                &Command {
                    receipt_id: receipt_id.clone(),
                    idempotency_key: idempotency_key.into(),
                    host_epoch,
                    kind: "provision".into(),
                    payload: json!({}),
                },
                &Event {
                    cursor: 0,
                    event_id: format!("accepted-{idempotency_key}"),
                    receipt_id,
                    host_epoch,
                    kind: "accepted".into(),
                    payload: json!({}),
                },
            )
            .unwrap()
        {
            ReceiptClaim::Accepted(receipt) => receipt,
            ReceiptClaim::Existing(_) => panic!("new test receipt must be accepted"),
        }
    }
}
