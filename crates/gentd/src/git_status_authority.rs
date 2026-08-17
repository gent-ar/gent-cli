//! Validation token for a future fixed, read-only Git status composition.
//!
//! This is intentionally independent from the broader authority profile. The shipped daemon
//! does not load it, expose it through arguments, or advertise a Git capability.

/// Request for the only Git effect this milestone can prepare: fixed-argv status.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum GitStatusAuthorityRequest {
    /// The default observer state: no Git executor can be retained or invoked.
    #[default]
    Observer,
    /// A later reviewed composition presents an auditable approval reference.
    Approved(GitStatusApproval),
}

/// Evidence identity a future composition owner must review before enabling status.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GitStatusApproval {
    pub(crate) evidence_reference: String,
}

/// Validation failure before a status executor can become reachable.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum GitStatusAuthorityError {
    #[error("Git status approval requires a non-empty evidence reference")]
    MissingEvidenceReference,
}

/// Non-effectful result passed to the dormant composition edge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ValidatedGitStatusAuthority {
    Observer,
    Approved(GitStatusApproval),
}

/// Validates an explicit status-only authority request without resolving or invoking Git.
///
/// # Errors
/// Returns an error when an approval does not name auditable evidence.
pub(crate) fn validate_git_status_authority(
    request: GitStatusAuthorityRequest,
) -> Result<ValidatedGitStatusAuthority, GitStatusAuthorityError> {
    match request {
        GitStatusAuthorityRequest::Observer => Ok(ValidatedGitStatusAuthority::Observer),
        GitStatusAuthorityRequest::Approved(approval) => {
            if approval.evidence_reference.trim().is_empty() {
                return Err(GitStatusAuthorityError::MissingEvidenceReference);
            }
            Ok(ValidatedGitStatusAuthority::Approved(approval))
        }
    }
}
