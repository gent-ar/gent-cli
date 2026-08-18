//! Pure attachment transfer validation; no filesystem, database, or transport imports.

use gent_types::{AttachmentMetadata, AttachmentState, AttachmentTransfer};

/// Maximum accepted byte length for one locally staged attachment.
pub const MAX_ATTACHMENT_BYTES: u64 = 64 * 1024 * 1024;
/// Maximum decoded byte count accepted by one IPC chunk.
pub const MAX_ATTACHMENT_CHUNK_BYTES: u64 = 256 * 1024;

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum AttachmentError {
    #[error("attachment metadata is invalid: {0}")]
    Metadata(&'static str),
    #[error("attachment transfer is not accepting chunks")]
    NotUploading,
    #[error("chunk offset does not match durable progress")]
    Offset,
    #[error("chunk exceeds the transfer bounds")]
    Bounds,
    #[error("attachment digest does not match the immutable metadata")]
    Digest,
}

/// Validates metadata before any adapter writes a byte.
///
/// # Errors
/// Returns an error for unsafe names, non-canonical digests, or non-content-addressed keys.
pub fn validate_attachment(metadata: &AttachmentMetadata) -> Result<(), AttachmentError> {
    if metadata.attachment_id.is_empty() || metadata.attachment_id.len() > 128 {
        return Err(AttachmentError::Metadata("attachment id"));
    }
    if metadata.display_name.is_empty()
        || metadata.display_name.len() > 255
        || metadata.display_name.contains(['/', '\\'])
        || metadata.display_name.chars().any(char::is_control)
    {
        return Err(AttachmentError::Metadata("display name"));
    }
    if !valid_media_type(&metadata.media_type) {
        return Err(AttachmentError::Metadata("media type"));
    }
    if metadata.byte_len > MAX_ATTACHMENT_BYTES {
        return Err(AttachmentError::Metadata("byte length"));
    }
    if !valid_digest(&metadata.digest_sha256) {
        return Err(AttachmentError::Metadata("sha256 digest"));
    }
    if metadata.storage_key != format!("sha256/{}", metadata.digest_sha256) {
        return Err(AttachmentError::Metadata("storage key"));
    }
    Ok(())
}

/// Validates the transfer-owned staging identity separately from the final content address.
///
/// # Errors
/// Returns an error when the staging identity is not a compact opaque token.
pub fn validate_staging_key(value: &str) -> Result<(), AttachmentError> {
    let Some(token) = value.strip_prefix("staging/") else {
        return Err(AttachmentError::Metadata("staging key"));
    };
    if token.is_empty()
        || token.len() > 128
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(AttachmentError::Metadata("staging key"));
    }
    Ok(())
}

/// Checks one sequential chunk without inspecting its bytes.
///
/// # Errors
/// Returns an error for a stale offset, non-uploading state, or size overrun.
pub fn accept_chunk(
    transfer: &AttachmentTransfer,
    offset: u64,
    byte_len: u64,
) -> Result<AttachmentTransfer, AttachmentError> {
    if transfer.state != AttachmentState::Uploading {
        return Err(AttachmentError::NotUploading);
    }
    if offset != transfer.received_bytes {
        return Err(AttachmentError::Offset);
    }
    if byte_len == 0 || byte_len > MAX_ATTACHMENT_CHUNK_BYTES {
        return Err(AttachmentError::Bounds);
    }
    let received_bytes = offset
        .checked_add(byte_len)
        .ok_or(AttachmentError::Bounds)?;
    if received_bytes > transfer.metadata.byte_len {
        return Err(AttachmentError::Bounds);
    }
    let mut next = transfer.clone();
    next.received_bytes = received_bytes;
    Ok(next)
}

/// Settles a fully received transfer after an adapter independently computes its SHA-256 digest.
///
/// # Errors
/// Returns an error unless every promised byte arrived and the digest exactly matches.
pub fn commit(
    transfer: &AttachmentTransfer,
    observed_digest: &str,
) -> Result<AttachmentTransfer, AttachmentError> {
    if transfer.state != AttachmentState::Uploading
        || transfer.received_bytes != transfer.metadata.byte_len
    {
        return Err(AttachmentError::NotUploading);
    }
    if observed_digest != transfer.metadata.digest_sha256 {
        return Err(AttachmentError::Digest);
    }
    let mut next = transfer.clone();
    next.state = AttachmentState::Available;
    Ok(next)
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_media_type(value: &str) -> bool {
    let Some((kind, subtype)) = value.split_once('/') else {
        return false;
    };
    !kind.is_empty()
        && !subtype.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'+' | b'-'))
}

#[cfg(test)]
mod tests {
    use gent_types::{
        AttachmentMetadata, AttachmentState, AttachmentTransfer, HostEpoch, ReceiptId,
    };

    use super::{
        AttachmentError, MAX_ATTACHMENT_BYTES, accept_chunk, commit, validate_attachment,
        validate_staging_key,
    };

    fn transfer() -> AttachmentTransfer {
        AttachmentTransfer {
            metadata: AttachmentMetadata {
                attachment_id: "attachment-1".into(),
                display_name: "notes.txt".into(),
                media_type: "text/plain".into(),
                byte_len: 4,
                digest_sha256: "a".repeat(64),
                storage_key: format!("sha256/{}", "a".repeat(64)),
            },
            staging_key: "staging/attachment-1".into(),
            receipt_id: ReceiptId("receipt-1".into()),
            idempotency_key: "attachment-1".into(),
            host_epoch: HostEpoch(1),
            state: AttachmentState::Uploading,
            received_bytes: 0,
        }
    }

    #[test]
    fn metadata_never_accepts_a_path_or_noncanonical_key() {
        let mut value = transfer().metadata;
        value.display_name = "../secret".into();
        assert!(validate_attachment(&value).is_err());
        value.display_name = "notes.txt".into();
        value.storage_key = "tmp/notes".into();
        assert!(validate_attachment(&value).is_err());
        assert!(validate_staging_key("/tmp/notes").is_err());
        assert!(validate_staging_key("staging/attachment-1").is_ok());
    }

    #[test]
    fn chunks_are_ordered_and_commit_is_terminal() {
        let transfer = accept_chunk(&transfer(), 0, 4).unwrap();
        assert_eq!(accept_chunk(&transfer, 0, 1), Err(AttachmentError::Offset));
        let committed = commit(&transfer, &"a".repeat(64)).unwrap();
        assert_eq!(committed.state, AttachmentState::Available);
        assert_eq!(
            accept_chunk(&committed, 4, 1),
            Err(AttachmentError::NotUploading)
        );
    }

    #[test]
    fn transfer_rejects_invalid_metadata_and_chunk_bounds() {
        let cases = [
            ("", "attachment id"),
            ("a/b", "display name"),
            ("notes.txt", "media type"),
        ];
        for (display_name, expected) in cases {
            let mut metadata = transfer().metadata;
            if expected == "attachment id" {
                metadata.attachment_id.clear();
            } else if expected == "display name" {
                metadata.display_name = display_name.into();
            } else {
                metadata.media_type = "text".into();
            }
            assert_eq!(
                validate_attachment(&metadata),
                Err(AttachmentError::Metadata(expected))
            );
        }
        assert_eq!(
            accept_chunk(&transfer(), 0, 0),
            Err(AttachmentError::Bounds)
        );
        assert_eq!(
            accept_chunk(&transfer(), 0, 5),
            Err(AttachmentError::Bounds)
        );
    }

    #[test]
    fn commit_requires_every_byte_and_the_exact_digest() {
        assert_eq!(
            commit(&transfer(), &"a".repeat(64)),
            Err(AttachmentError::NotUploading)
        );
        let complete = accept_chunk(&transfer(), 0, 4).unwrap();
        assert_eq!(commit(&complete, "b"), Err(AttachmentError::Digest));
        assert!(validate_staging_key("staging/contains_underscore").is_err());
    }

    #[test]
    fn metadata_limits_digest_and_content_key_are_fenced() {
        let mut metadata = transfer().metadata;
        metadata.byte_len = MAX_ATTACHMENT_BYTES + 1;
        assert_eq!(
            validate_attachment(&metadata),
            Err(AttachmentError::Metadata("byte length"))
        );
        metadata.byte_len = 4;
        metadata.digest_sha256 = "A".repeat(64);
        assert_eq!(
            validate_attachment(&metadata),
            Err(AttachmentError::Metadata("sha256 digest"))
        );
        metadata.digest_sha256 = "a".repeat(64);
        metadata.storage_key = "sha256/other".into();
        assert_eq!(
            validate_attachment(&metadata),
            Err(AttachmentError::Metadata("storage key"))
        );
    }
}
