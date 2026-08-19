//! Bounded file/key parsing helpers for the signed ordinary-authority release.
use super::{
    MAX_BYTES, MAX_KEYS, OrdinaryAuthorityReleaseError, OrdinaryAuthorityReleasePayload,
    ReleaseVerificationKey,
};
use ed25519_dalek::VerifyingKey;
use gent_adapters::compatibility::TrustedKeySet;
use std::{collections::BTreeSet, fs, path::Path};

pub(super) fn read(path: &Path) -> Result<Vec<u8>, OrdinaryAuthorityReleaseError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| OrdinaryAuthorityReleaseError::Unavailable)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(OrdinaryAuthorityReleaseError::NotRegular);
    }
    if metadata.len() > MAX_BYTES {
        return Err(OrdinaryAuthorityReleaseError::TooLarge);
    }
    let bytes = fs::read(path).map_err(|_| OrdinaryAuthorityReleaseError::Unavailable)?;
    (bytes.len() as u64 <= MAX_BYTES)
        .then_some(bytes)
        .ok_or(OrdinaryAuthorityReleaseError::TooLarge)
}

pub(super) fn keys(
    values: &[ReleaseVerificationKey],
) -> Result<TrustedKeySet, OrdinaryAuthorityReleaseError> {
    if values.is_empty() || values.len() > MAX_KEYS {
        return Err(OrdinaryAuthorityReleaseError::EmbeddedAuthority);
    }
    let mut ids = BTreeSet::new();
    let mut result = TrustedKeySet::default();
    for value in values {
        let bytes = hex::decode(&value.public_key_hex)
            .map_err(|_| OrdinaryAuthorityReleaseError::EmbeddedAuthority)?;
        if !valid_id(&value.key_id)
            || !valid_hex(&value.public_key_hex)
            || bytes.len() != 32
            || !ids.insert(&value.key_id)
        {
            return Err(OrdinaryAuthorityReleaseError::EmbeddedAuthority);
        }
        result.trust(
            &value.key_id,
            VerifyingKey::from_bytes(
                bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| OrdinaryAuthorityReleaseError::EmbeddedAuthority)?,
            )
            .map_err(|_| OrdinaryAuthorityReleaseError::EmbeddedAuthority)?,
        );
    }
    Ok(result)
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

pub(super) fn valid_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

pub(super) fn canonical_payload(
    payload: &OrdinaryAuthorityReleasePayload,
) -> Result<Vec<u8>, OrdinaryAuthorityReleaseError> {
    serde_json::to_vec(
        &serde_json::to_value(payload).map_err(|_| OrdinaryAuthorityReleaseError::Malformed)?,
    )
    .map_err(|_| OrdinaryAuthorityReleaseError::Malformed)
}
