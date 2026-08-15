//! Adapter joining opaque attachment transfer metadata to `SQLite`.

use gent_ports::{AttachmentClaim, AttachmentLedger, IngressMode, LedgerError};
use gent_types::{AttachmentState, AttachmentTransfer, HostEpoch, ReceiptId, TurnAttachment};
use rusqlite::{OptionalExtension, params};

use super::SqliteLedger;
use super::queries::{host_ingress, storage_error};

impl AttachmentLedger for SqliteLedger {
    fn claim_attachment(
        &self,
        transfer: &AttachmentTransfer,
    ) -> Result<AttachmentClaim, LedgerError> {
        let connection = self.lock()?;
        guard_ingress(&connection, transfer.host_epoch)?;
        if let Some(existing) = find(&connection, &transfer.metadata.attachment_id)? {
            return if same_identity(&existing, transfer) {
                Ok(AttachmentClaim::Existing(existing))
            } else {
                Err(LedgerError::Invariant(
                    "attachment id has conflicting immutable metadata".into(),
                ))
            };
        }
        if let Some(existing) = find_by_key(&connection, &transfer.idempotency_key)? {
            return if same_identity(&existing, transfer) {
                Ok(AttachmentClaim::Existing(existing))
            } else {
                Err(LedgerError::Invariant(
                    "attachment idempotency key conflicts".into(),
                ))
            };
        }
        connection.execute(
            "INSERT INTO attachments (attachment_id, idempotency_key, receipt_id, host_epoch, state, received_bytes, display_name, media_type, byte_len, digest_sha256, storage_key) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![transfer.metadata.attachment_id, transfer.idempotency_key, transfer.receipt_id.0, transfer.host_epoch.0, state(transfer.state), transfer.received_bytes, transfer.metadata.display_name, transfer.metadata.media_type, transfer.metadata.byte_len, transfer.metadata.digest_sha256, transfer.metadata.storage_key],
        ).map_err(storage_error)?;
        Ok(AttachmentClaim::Created(transfer.clone()))
    }

    fn replace_attachment(
        &self,
        expected: &AttachmentTransfer,
        next: &AttachmentTransfer,
    ) -> Result<(), LedgerError> {
        if expected.metadata != next.metadata
            || expected.receipt_id != next.receipt_id
            || expected.idempotency_key != next.idempotency_key
            || expected.host_epoch != next.host_epoch
        {
            return Err(LedgerError::Invariant(
                "attachment metadata is immutable".into(),
            ));
        }
        let connection = self.lock()?;
        guard_ingress(&connection, expected.host_epoch)?;
        let changed = connection.execute(
            "UPDATE attachments SET state = ?1, received_bytes = ?2 WHERE attachment_id = ?3 AND state = ?4 AND received_bytes = ?5",
            params![state(next.state), next.received_bytes, expected.metadata.attachment_id, state(expected.state), expected.received_bytes],
        ).map_err(storage_error)?;
        if changed == 1 {
            Ok(())
        } else {
            Err(LedgerError::Invariant(
                "attachment changed concurrently or is missing".into(),
            ))
        }
    }

    fn find_attachment(
        &self,
        attachment_id: &str,
    ) -> Result<Option<AttachmentTransfer>, LedgerError> {
        let connection = self.lock()?;
        find(&connection, attachment_id)
    }

    fn attach_to_turn(&self, association: &TurnAttachment) -> Result<(), LedgerError> {
        let connection = self.lock()?;
        let available = connection
            .query_row(
                "SELECT state FROM attachments WHERE attachment_id = ?1",
                [&association.attachment_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage_error)?;
        if available.as_deref() != Some("available") {
            return Err(LedgerError::Invariant(
                "only available attachments can be associated".into(),
            ));
        }
        connection
            .execute(
                "INSERT OR IGNORE INTO turn_attachments (turn_id, attachment_id) VALUES (?1, ?2)",
                params![association.turn_id, association.attachment_id],
            )
            .map(|_| ())
            .map_err(storage_error)
    }
}

fn find(
    connection: &rusqlite::Connection,
    attachment_id: &str,
) -> Result<Option<AttachmentTransfer>, LedgerError> {
    connection.query_row("SELECT attachment_id, display_name, media_type, byte_len, digest_sha256, storage_key, receipt_id, idempotency_key, host_epoch, state, received_bytes FROM attachments WHERE attachment_id = ?1", [attachment_id], decode).optional().map_err(storage_error)
}
fn find_by_key(
    connection: &rusqlite::Connection,
    key: &str,
) -> Result<Option<AttachmentTransfer>, LedgerError> {
    connection.query_row("SELECT attachment_id, display_name, media_type, byte_len, digest_sha256, storage_key, receipt_id, idempotency_key, host_epoch, state, received_bytes FROM attachments WHERE idempotency_key = ?1", [key], decode).optional().map_err(storage_error)
}
fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<AttachmentTransfer> {
    Ok(AttachmentTransfer {
        metadata: gent_types::AttachmentMetadata {
            attachment_id: row.get(0)?,
            display_name: row.get(1)?,
            media_type: row.get(2)?,
            byte_len: row.get(3)?,
            digest_sha256: row.get(4)?,
            storage_key: row.get(5)?,
        },
        receipt_id: ReceiptId(row.get(6)?),
        idempotency_key: row.get(7)?,
        host_epoch: HostEpoch(row.get(8)?),
        state: match row.get::<_, String>(9)?.as_str() {
            "uploading" => AttachmentState::Uploading,
            "available" => AttachmentState::Available,
            "rejected" => AttachmentState::Rejected,
            _ => return Err(rusqlite::Error::InvalidQuery),
        },
        received_bytes: row.get(10)?,
    })
}
fn same_identity(left: &AttachmentTransfer, right: &AttachmentTransfer) -> bool {
    left.metadata == right.metadata
        && left.receipt_id == right.receipt_id
        && left.idempotency_key == right.idempotency_key
        && left.host_epoch == right.host_epoch
}
fn guard_ingress(connection: &rusqlite::Connection, epoch: HostEpoch) -> Result<(), LedgerError> {
    let ingress = host_ingress(connection)?;
    if ingress.epoch != epoch {
        return Err(LedgerError::StaleEpoch {
            command: epoch,
            active: ingress.epoch,
        });
    }
    if ingress.mode == IngressMode::Closed {
        return Err(LedgerError::IngressClosed { epoch });
    }
    Ok(())
}
const fn state(state: AttachmentState) -> &'static str {
    match state {
        AttachmentState::Uploading => "uploading",
        AttachmentState::Available => "available",
        AttachmentState::Rejected => "rejected",
    }
}
