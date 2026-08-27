use gent_types::ConversationContentEntry;
use sha2::{Digest, Sha256};

pub(super) fn digest_matches(text: &str, expected: &str) -> bool {
    valid_digest(expected) && format!("{:x}", Sha256::digest(text.as_bytes())) == expected
}
pub(super) fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
pub(super) fn digest_entries(entries: &[ConversationContentEntry]) -> String {
    let mut hasher = Sha256::new();
    for entry in entries {
        hasher.update(entry.ordinal.to_be_bytes());
        hasher.update(entry.text_digest_sha256.as_bytes());
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}
