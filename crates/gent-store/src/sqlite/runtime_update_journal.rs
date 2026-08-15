//! `SQLite` persistence for append-only runtime-update checkpoints.

use gent_ports::LedgerError;
use gent_ports::runtime_update::RuntimeUpdateJournal;
use gent_types::RuntimeUpdateRecord;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::SqliteLedger;
use super::queries::storage_error;

impl RuntimeUpdateJournal for SqliteLedger {
    fn save_runtime_update(&self, record: &RuntimeUpdateRecord) -> Result<(), LedgerError> {
        validate(record)?;
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        if let Some(existing) = find_revision(&transaction, &record.attempt_id, record.revision)? {
            if existing == *record {
                return Ok(());
            }
            return Err(LedgerError::Invariant(
                "runtime update revision conflicts with existing state".into(),
            ));
        }
        if let Some(current) = find(&transaction, &record.attempt_id)?
            && record.revision <= current.revision
        {
            return Err(LedgerError::Invariant(
                "runtime update revision regressed".into(),
            ));
        }
        transaction
            .execute(
                "INSERT INTO runtime_update_journal (attempt_id, revision, artifact_digest_sha256, payload) VALUES (?1, ?2, ?3, ?4)",
                params![record.attempt_id, record.revision, record.artifact_digest_sha256, encode(record)?],
            )
            .map_err(storage_error)?;
        transaction.commit().map_err(storage_error)
    }

    fn find_runtime_update(
        &self,
        attempt_id: &str,
    ) -> Result<Option<RuntimeUpdateRecord>, LedgerError> {
        let connection = self.lock()?;
        find(&connection, attempt_id)
    }
}

fn validate(record: &RuntimeUpdateRecord) -> Result<(), LedgerError> {
    if record.revision == 0 || record.artifact_digest_sha256.len() != 64 {
        return Err(LedgerError::Invariant(
            "runtime update requires a nonzero revision and SHA-256 digest".into(),
        ));
    }
    Ok(())
}

fn find(
    connection: &Connection,
    attempt_id: &str,
) -> Result<Option<RuntimeUpdateRecord>, LedgerError> {
    connection
        .query_row(
            "SELECT payload FROM runtime_update_journal WHERE attempt_id = ?1 ORDER BY revision DESC LIMIT 1",
            [attempt_id],
            |row| decode(&row.get::<_, String>(0)?),
        )
        .optional()
        .map_err(storage_error)
}

fn find_revision(
    connection: &Connection,
    attempt_id: &str,
    revision: u64,
) -> Result<Option<RuntimeUpdateRecord>, LedgerError> {
    connection
        .query_row(
            "SELECT payload FROM runtime_update_journal WHERE attempt_id = ?1 AND revision = ?2",
            params![attempt_id, revision],
            |row| decode(&row.get::<_, String>(0)?),
        )
        .optional()
        .map_err(storage_error)
}

fn decode(payload: &str) -> rusqlite::Result<RuntimeUpdateRecord> {
    serde_json::from_str(payload).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}

fn encode(record: &RuntimeUpdateRecord) -> Result<String, LedgerError> {
    serde_json::to_string(record).map_err(storage_error)
}
