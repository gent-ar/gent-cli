//! Daemon-side canonical workspace identity derivation.

use std::path::Path;

use gent_types::WorkspaceRecord;
use sha2::{Digest, Sha256};

/// One selected workspace after daemon-side canonicalization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CanonicalWorkspace {
    record: WorkspaceRecord,
}

impl CanonicalWorkspace {
    /// Canonicalizes one existing directory and derives its stable Gent identity.
    ///
    /// # Errors
    /// Returns an error when the path is not an accessible directory or cannot be represented.
    pub(crate) fn from_path(path: &Path) -> Result<Self, WorkspaceIdentityError> {
        let canonical_path = path
            .canonicalize()
            .map_err(|_| WorkspaceIdentityError::Unavailable)?;
        if !canonical_path.is_dir() {
            return Err(WorkspaceIdentityError::NotDirectory);
        }
        let canonical_path = canonical_path
            .to_str()
            .filter(|value| !value.is_empty() && !value.contains('\0'))
            .ok_or(WorkspaceIdentityError::InvalidPath)?
            .to_owned();
        Ok(Self {
            record: WorkspaceRecord {
                workspace_id: workspace_id(&canonical_path),
                canonical_path,
            },
        })
    }

    /// Returns the durable workspace record for an atomic chat-binding transaction.
    #[must_use]
    pub(crate) fn record(&self) -> &WorkspaceRecord {
        &self.record
    }
}

/// Failure before a client path becomes a durable workspace identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkspaceIdentityError {
    Unavailable,
    NotDirectory,
    InvalidPath,
}

fn workspace_id(canonical_path: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"gent-workspace-v1\0");
    digest.update(canonical_path.as_bytes());
    format!("workspace-{:x}", digest.finalize())
}
