use gent_ports::LedgerError;
use rusqlite::{OptionalExtension, Transaction, params};

use crate::sqlite::queries::storage_error;

pub(super) fn attach_available(
    transaction: &Transaction<'_>,
    turn_id: &str,
    attachment_ids: &[String],
) -> Result<(), LedgerError> {
    for attachment_id in attachment_ids {
        let available = transaction
            .query_row(
                "SELECT state FROM attachments WHERE attachment_id = ?1",
                [attachment_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage_error)?;
        if available.as_deref() != Some("available") {
            return Err(LedgerError::Invariant(
                "prompt attachment is unavailable".into(),
            ));
        }
        transaction
            .execute(
                "INSERT INTO turn_attachments (turn_id, attachment_id) VALUES (?1, ?2)",
                params![turn_id, attachment_id],
            )
            .map_err(storage_error)?;
    }
    Ok(())
}
