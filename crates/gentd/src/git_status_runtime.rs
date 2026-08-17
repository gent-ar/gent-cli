//! Dormant composition edge for one reviewed, read-only Git status service.
//!
//! `main` never constructs this type. The shipped daemon is observer-only, so neither this
//! runtime nor a Git executor is reachable through normal daemon startup or local IPC.

#![allow(dead_code)] // The shipped observer intentionally retains no Git authority path.

#[path = "git_status_authority.rs"]
mod git_status_authority;
#[cfg(test)]
#[path = "git_status_runtime_tests.rs"]
mod git_status_runtime_tests;

use gent_ports::GitExecutor;
use gent_runtime::{GitStatusAuthority, GitStatusService};

use self::git_status_authority::ValidatedGitStatusAuthority;

/// Refuses to construct the status runtime from an observer authority token.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum GitStatusRuntimeError {
    #[error("the authority profile does not approve read-only Git status")]
    StatusAuthorityUnavailable,
}

/// Fully injected fixed-status composition for a later reviewed daemon profile.
#[derive(Debug)]
pub(crate) struct PreparedGitStatusRuntime<L, E> {
    service: GitStatusService<L, E>,
}

impl<L, E: GitExecutor> PreparedGitStatusRuntime<L, E> {
    /// Binds caller-supplied durable and OS-edge ports only after status approval.
    ///
    /// # Errors
    /// Returns before retaining either port unless the explicit token approves Git status.
    pub(crate) fn new(
        authority: &ValidatedGitStatusAuthority,
        ledger: L,
        executor: E,
    ) -> Result<Self, GitStatusRuntimeError> {
        if !matches!(authority, ValidatedGitStatusAuthority::Approved(_)) {
            return Err(GitStatusRuntimeError::StatusAuthorityUnavailable);
        }
        Ok(Self {
            service: GitStatusService::new(ledger, executor, GitStatusAuthority::ApprovedStatus),
        })
    }

    /// Returns the sole fixed, read-only status service made reachable by this token.
    #[must_use]
    pub(crate) const fn service(&self) -> &GitStatusService<L, E> {
        &self.service
    }
}
