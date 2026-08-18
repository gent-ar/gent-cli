//! Bounded, non-effectful input handling shared by dormant provider evidence preflights.

use std::{collections::BTreeSet, fs, path::Path};

use ed25519_dalek::VerifyingKey;
use gent_adapters::compatibility::TrustedKeySet;
use serde::de::DeserializeOwned;

const MAX_RECORD_BYTES: u64 = 65_536;
const MAX_KEY_ID_BYTES: usize = 128;

#[derive(Debug)]
pub(crate) enum AuthorityEvidenceInputError {
    Unavailable,
    NotRegular,
    TooLarge,
    Unreadable,
    Malformed,
    MissingTrustedKey,
    InvalidKey,
    DuplicateKey,
}

pub(crate) fn read_record<T: DeserializeOwned>(
    path: &Path,
) -> Result<T, AuthorityEvidenceInputError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| AuthorityEvidenceInputError::Unavailable)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AuthorityEvidenceInputError::NotRegular);
    }
    if metadata.len() > MAX_RECORD_BYTES {
        return Err(AuthorityEvidenceInputError::TooLarge);
    }
    let bytes = fs::read(path).map_err(|_| AuthorityEvidenceInputError::Unreadable)?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(AuthorityEvidenceInputError::TooLarge);
    }
    serde_json::from_slice(&bytes).map_err(|_| AuthorityEvidenceInputError::Malformed)
}

pub(crate) fn parse_keys(values: &[String]) -> Result<TrustedKeySet, AuthorityEvidenceInputError> {
    if values.is_empty() {
        return Err(AuthorityEvidenceInputError::MissingTrustedKey);
    }
    let mut ids = BTreeSet::new();
    let mut keys = TrustedKeySet::default();
    for value in values {
        let (key_id, encoded) = value
            .split_once(':')
            .ok_or(AuthorityEvidenceInputError::InvalidKey)?;
        if !valid_key_id(key_id) || encoded.len() != 64 || !encoded.bytes().all(lower_hex) {
            return Err(AuthorityEvidenceInputError::InvalidKey);
        }
        if !ids.insert(key_id) {
            return Err(AuthorityEvidenceInputError::DuplicateKey);
        }
        let bytes = hex::decode(encoded).map_err(|_| AuthorityEvidenceInputError::InvalidKey)?;
        let key = VerifyingKey::from_bytes(
            bytes
                .as_slice()
                .try_into()
                .map_err(|_| AuthorityEvidenceInputError::InvalidKey)?,
        )
        .map_err(|_| AuthorityEvidenceInputError::InvalidKey)?;
        keys.trust(key_id, key);
    }
    Ok(keys)
}

fn valid_key_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_KEY_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')
}
