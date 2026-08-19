//! Append-only trusted public-provider installation locks.

use gent_ports::{LedgerError, ProvisionedProviderLockLedger};
use gent_types::{
    Event, ProviderInstallProvenance, ProvisionedProviderInstallation, ProvisionedProviderLock,
    Receipt, ReceiptStatus, RunVersionLock,
};
use rusqlite::{OptionalExtension, params};

use super::{
    SqliteLedger,
    queries::{append_event, encode_status, find_receipt, storage_error},
};

impl ProvisionedProviderLockLedger for SqliteLedger {
    fn settle_provisioned_provider_provision(
        &self,
        receipt: &Receipt,
        installation: Option<&ProvisionedProviderInstallation>,
        status: ReceiptStatus,
        terminal: &Event,
    ) -> Result<Receipt, LedgerError> {
        require_terminal(receipt, installation, &status, terminal)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let mut durable = find_receipt(&transaction, &receipt.idempotency_key)?
            .ok_or_else(|| LedgerError::Invariant("provisioning receipt is missing".into()))?;
        if durable.receipt_id != receipt.receipt_id
            || durable.idempotency_key != receipt.idempotency_key
            || durable.host_epoch != receipt.host_epoch
        {
            return Err(LedgerError::Invariant(
                "provisioning receipt no longer exactly matches".into(),
            ));
        }
        if durable.status != ReceiptStatus::Accepted {
            return existing_settlement(&transaction, &durable, installation, &status);
        }
        if let Some(installation) = installation {
            save_installation(&transaction, receipt, installation)?;
        }
        transaction
            .execute(
                "UPDATE receipts SET status = ?1 WHERE idempotency_key = ?2",
                params![encode_status(&status), receipt.idempotency_key],
            )
            .map_err(storage_error)?;
        append_event(&transaction, terminal)?;
        transaction.commit().map_err(storage_error)?;
        durable.status = status;
        Ok(durable)
    }

    fn find_provisioned_provider_installation(
        &self,
        provider: &str,
    ) -> Result<Option<ProvisionedProviderInstallation>, LedgerError> {
        self.lock()?
            .query_row(
                "SELECT provider, canonical_path, file_identity, digest_sha256, version, compatibility_entry, package_name, package_version, package_integrity, package_policy_digest_sha256, node_runtime_digest_sha256 FROM provisioned_provider_locks WHERE provider = ?1 ORDER BY installation_ordinal DESC LIMIT 1",
                [provider],
                read_installation,
            )
            .optional()
            .map_err(storage_error)
    }
}

fn require_terminal(
    receipt: &Receipt,
    installation: Option<&ProvisionedProviderInstallation>,
    status: &ReceiptStatus,
    terminal: &Event,
) -> Result<(), LedgerError> {
    (receipt.status == ReceiptStatus::Accepted
        && *status != ReceiptStatus::Accepted
        && terminal.receipt_id == receipt.receipt_id
        && terminal.host_epoch == receipt.host_epoch
        && installation.is_some() == (*status == ReceiptStatus::Settled))
        .then_some(())
        .ok_or_else(|| LedgerError::Invariant("invalid provisioning settlement".into()))
}

fn existing_settlement(
    connection: &rusqlite::Connection,
    receipt: &Receipt,
    installation: Option<&ProvisionedProviderInstallation>,
    status: &ReceiptStatus,
) -> Result<Receipt, LedgerError> {
    if receipt.status != *status {
        return Err(LedgerError::Invariant(
            "provisioning receipt already has a different terminal state".into(),
        ));
    }
    match installation {
        Some(installation) => (find_by_receipt(connection, &receipt.receipt_id.0)?
            == Some(installation.clone()))
        .then_some(receipt.clone())
        .ok_or_else(|| LedgerError::Invariant("provisioned provider lock is immutable".into())),
        None => Ok(receipt.clone()),
    }
}

fn save_installation(
    connection: &rusqlite::Connection,
    receipt: &Receipt,
    installation: &ProvisionedProviderInstallation,
) -> Result<(), LedgerError> {
    if let Some(existing) = find_by_receipt(connection, &receipt.receipt_id.0)? {
        return (existing == *installation).then_some(()).ok_or_else(|| {
            LedgerError::Invariant("provisioned provider lock is immutable".into())
        });
    }
    require_provenance(&installation.provenance)?;
    connection
        .execute(
            "INSERT INTO provisioned_provider_locks (provider, receipt_id, idempotency_key, host_epoch, canonical_path, file_identity, digest_sha256, version, compatibility_entry, package_name, package_version, package_integrity, package_policy_digest_sha256, node_runtime_digest_sha256) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![installation.lock.run_lock.provider, receipt.receipt_id.0, receipt.idempotency_key, receipt.host_epoch.0, installation.lock.run_lock.canonical_path, installation.lock.run_lock.file_identity, installation.lock.run_lock.digest_sha256, installation.lock.run_lock.version, installation.lock.run_lock.compatibility_entry, installation.provenance.package_name, installation.provenance.package_version, installation.provenance.package_integrity, installation.provenance.package_policy_digest_sha256, installation.provenance.node_runtime_digest_sha256],
        )
        .map(|_| ())
        .map_err(storage_error)
}

fn find_by_receipt(
    connection: &rusqlite::Connection,
    receipt_id: &str,
) -> Result<Option<ProvisionedProviderInstallation>, LedgerError> {
    connection
        .query_row(
            "SELECT provider, canonical_path, file_identity, digest_sha256, version, compatibility_entry, package_name, package_version, package_integrity, package_policy_digest_sha256, node_runtime_digest_sha256 FROM provisioned_provider_locks WHERE receipt_id = ?1",
            [receipt_id],
            read_installation,
        )
        .optional()
        .map_err(storage_error)
}

fn read_installation(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProvisionedProviderInstallation> {
    Ok(ProvisionedProviderInstallation {
        lock: ProvisionedProviderLock {
            run_lock: RunVersionLock {
                provider: row.get(0)?,
                canonical_path: row.get(1)?,
                file_identity: row.get(2)?,
                digest_sha256: row.get(3)?,
                version: row.get(4)?,
                compatibility_entry: row.get(5)?,
            },
        },
        provenance: ProviderInstallProvenance {
            package_name: row.get(6)?,
            package_version: row.get(7)?,
            package_integrity: row.get(8)?,
            package_policy_digest_sha256: row.get(9)?,
            node_runtime_digest_sha256: row.get(10)?,
        },
    })
}

fn require_provenance(provenance: &ProviderInstallProvenance) -> Result<(), LedgerError> {
    let exact_package = !provenance.package_name.is_empty()
        && !provenance.package_version.is_empty()
        && provenance.package_version.trim() == provenance.package_version
        && !provenance.package_integrity.is_empty();
    (exact_package
        && is_sha256(&provenance.package_policy_digest_sha256)
        && is_sha256(&provenance.node_runtime_digest_sha256))
    .then_some(())
    .ok_or_else(|| LedgerError::Invariant("provider installation provenance is invalid".into()))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
#[path = "provisioned_provider_locks_tests.rs"]
mod tests;
