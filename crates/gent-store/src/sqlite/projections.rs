//! `SQLite` persistence for complete, cursor-monotonic run lifecycle projections.

use gent_ports::{LedgerError, RunProjectionLedger};
use gent_types::{HostEpoch, RunProjectionRecord};
use rusqlite::{Connection, OptionalExtension, params};

use super::SqliteLedger;
use super::queries::{find_run, storage_error};

impl RunProjectionLedger for SqliteLedger {
    fn save_run_projection(&self, record: &RunProjectionRecord) -> Result<(), LedgerError> {
        let connection = self.lock()?;
        save(&connection, record)
    }

    fn find_run_projection(
        &self,
        run_id: &str,
    ) -> Result<Option<RunProjectionRecord>, LedgerError> {
        let connection = self.lock()?;
        find(&connection, run_id)
    }
}

pub(super) fn save(
    connection: &Connection,
    record: &RunProjectionRecord,
) -> Result<(), LedgerError> {
    if find_run(connection, &record.run_id)?.is_none() {
        return Err(LedgerError::Invariant(
            "projection run does not exist".into(),
        ));
    }
    let existing = find(connection, &record.run_id)?;
    if let Some(existing) = existing {
        if existing.projection.cursor > record.projection.cursor {
            return Err(LedgerError::Invariant("projection cursor regressed".into()));
        }
        if existing.projection.cursor == record.projection.cursor {
            return if existing == *record {
                Ok(())
            } else {
                Err(LedgerError::Invariant(
                    "projection cursor conflicts with existing state".into(),
                ))
            };
        }
        connection
            .execute(
                "UPDATE run_projections SET host_epoch = ?1, cursor = ?2, payload = ?3 WHERE run_id = ?4",
                params![record.host_epoch.0, record.projection.cursor, encode(record)?, record.run_id],
            )
            .map(|_| ())
            .map_err(storage_error)
    } else {
        connection
            .execute(
                "INSERT INTO run_projections (run_id, host_epoch, cursor, payload) VALUES (?1, ?2, ?3, ?4)",
                params![record.run_id, record.host_epoch.0, record.projection.cursor, encode(record)?],
            )
            .map(|_| ())
            .map_err(storage_error)
    }
}

pub(super) fn find(
    connection: &Connection,
    run_id: &str,
) -> Result<Option<RunProjectionRecord>, LedgerError> {
    connection
        .query_row(
            "SELECT host_epoch, payload FROM run_projections WHERE run_id = ?1",
            [run_id],
            |row| {
                let payload = row.get::<_, String>(1)?;
                let mut record =
                    serde_json::from_str::<RunProjectionRecord>(&payload).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?;
                record.run_id = run_id.into();
                record.host_epoch = HostEpoch(row.get(0)?);
                Ok(record)
            },
        )
        .optional()
        .map_err(storage_error)
}

fn encode(record: &RunProjectionRecord) -> Result<String, LedgerError> {
    serde_json::to_string(record).map_err(storage_error)
}
