//! Narrow, typed boundary for daemon-owned Git status execution.

use gent_types::{WorkspaceGitFileStatus, WorkspaceGitWorktree};

/// A status operation whose worktree path was selected by the daemon, never a client argv string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitStatusOperation {
    pub canonical_worktree_path: String,
}

/// The richer git report for one already-resolved repository root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitReport {
    pub branch: Option<String>,
    pub files: Vec<WorkspaceGitFileStatus>,
    pub worktrees: Vec<WorkspaceGitWorktree>,
}

/// Content-safe result from a bounded `git status --porcelain=v1 -z` invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitStatusSummary {
    pub entry_count: u32,
    pub branch_name: Option<String>,
    pub output_digest_sha256: String,
}

/// Controlled failures from a fixed Git status invocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum GitExecutorError {
    #[error("Git worktree path is invalid")]
    InvalidWorktree,
    #[error("Git could not be started")]
    SpawnFailed,
    #[error("Git status output exceeded the configured bound")]
    OutputTooLarge,
    #[error("Git status returned a failure")]
    StatusFailed,
    #[error("Git returned malformed porcelain status output")]
    InvalidOutput,
}

/// Executes fixed, read-only Git operations owned by the daemon.
pub trait GitExecutor: Send + Sync {
    /// Runs status for a canonical worktree without accepting a shell command or client argv.
    ///
    /// # Errors
    /// Returns a controlled failure without retaining Git output or standard error.
    fn status(&self, operation: &GitStatusOperation) -> Result<GitStatusSummary, GitExecutorError>;

    /// Resolves the repository root containing a canonical directory, which may be a subdirectory
    /// of the repository rather than its root.
    ///
    /// # Errors
    /// Returns a controlled failure when the directory is not inside a git repository.
    fn repository_root(&self, canonical_path: &str) -> Result<String, GitExecutorError>;

    /// Runs status, branch, and worktree list for an already-resolved repository root.
    ///
    /// # Errors
    /// Returns a controlled failure without retaining Git output or standard error.
    fn report(&self, canonical_repository_root: &str) -> Result<GitReport, GitExecutorError>;

    /// Restores each path to its checked-in content, matching `git checkout -- <path>...`.
    ///
    /// # Errors
    /// Returns a controlled failure when the checkout cannot complete.
    fn checkout_paths(
        &self,
        canonical_repository_root: &str,
        paths: &[String],
    ) -> Result<(), GitExecutorError>;
}
