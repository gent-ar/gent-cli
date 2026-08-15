//! Narrow, typed boundary for daemon-owned Git status execution.

/// A status operation whose worktree path was selected by the daemon, never a client argv string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitStatusOperation {
    pub canonical_worktree_path: String,
}

/// Content-safe result from a bounded `git status --porcelain=v1 -z` invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitStatusSummary {
    pub entry_count: u32,
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

/// Executes the one fixed, read-only Git status operation owned by the daemon.
pub trait GitExecutor: Send + Sync {
    /// Runs status for a canonical worktree without accepting a shell command or client argv.
    ///
    /// # Errors
    /// Returns a controlled failure without retaining Git output or standard error.
    fn status(&self, operation: &GitStatusOperation) -> Result<GitStatusSummary, GitExecutorError>;
}
