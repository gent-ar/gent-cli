//! `SQLite` persistence for complete capability snapshots.

use gent_ports::LedgerError;
use gent_types::{CapabilityCatalogRecord, CapabilitySet};
use rusqlite::OptionalExtension;

use super::SqliteLedger;
use super::queries::storage_error;

pub(super) fn save(
    ledger: &SqliteLedger,
    catalog: &CapabilityCatalogRecord,
) -> Result<(), LedgerError> {
    let capabilities = serde_json::to_string(&catalog.capabilities).map_err(storage_error)?;
    ledger.lock()?.execute(
        "INSERT INTO capability_catalog (singleton, schema_version, capabilities) VALUES (1, ?1, ?2) ON CONFLICT(singleton) DO UPDATE SET schema_version = excluded.schema_version, capabilities = excluded.capabilities",
        (i64::from(catalog.schema_version), capabilities),
    ).map(|_| ()).map_err(storage_error)
}

pub(super) fn load(ledger: &SqliteLedger) -> Result<Option<CapabilityCatalogRecord>, LedgerError> {
    ledger
        .lock()?
        .query_row(
            "SELECT schema_version, capabilities FROM capability_catalog WHERE singleton = 1",
            [],
            |row| {
                Ok(CapabilityCatalogRecord {
                    schema_version: u32::try_from(row.get::<_, i64>(0)?)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    capabilities: serde_json::from_str::<CapabilitySet>(&row.get::<_, String>(1)?)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?,
                })
            },
        )
        .optional()
        .map_err(storage_error)
}
