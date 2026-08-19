//! Deterministic content identity for one exact accepted [`Command`].

use sha2::{Digest, Sha256};

use crate::Command;

impl Command {
    /// Deterministic SHA-256 identity of this exact accepted command.
    ///
    /// Binds receipt correlation (`receipt_id`, `idempotency_key`, `host_epoch`) together with
    /// the command's own `kind` and `payload`, so a persisted result can be tied back to the
    /// exact command that produced it. Two commands with identical content but different
    /// receipt correlation never collide.
    #[must_use]
    pub fn receipt_fingerprint_sha256(&self) -> String {
        let mut hasher = Sha256::new();
        write_field(&mut hasher, "receiptId", &self.receipt_id.0);
        write_field(&mut hasher, "idempotencyKey", &self.idempotency_key);
        write_field(&mut hasher, "hostEpoch", &self.host_epoch.0.to_string());
        write_field(&mut hasher, "kind", &self.kind);
        write_field(
            &mut hasher,
            "payload",
            &serde_json::to_string(&self.payload).unwrap_or_default(),
        );
        format!("{:x}", hasher.finalize())
    }
}

fn write_field(hasher: &mut Sha256, name: &str, value: &str) {
    hasher.update(name.len().to_be_bytes());
    hasher.update(name.as_bytes());
    hasher.update(value.len().to_be_bytes());
    hasher.update(value.as_bytes());
}
