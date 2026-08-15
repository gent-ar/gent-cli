//! Fixed-argv, bounded operating-system edge for read-only Git status.

use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
};

use gent_ports::{GitExecutor, GitExecutorError, GitStatusOperation, GitStatusSummary};
use sha2::{Digest, Sha256};

use crate::parse_porcelain_v1_z;

const MAX_STATUS_BYTES: usize = 1_048_576;

/// The production implementation of the narrow read-only Git status port.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemGitExecutor;

impl GitExecutor for SystemGitExecutor {
    fn status(&self, operation: &GitStatusOperation) -> Result<GitStatusSummary, GitExecutorError> {
        let worktree = canonical_worktree(&operation.canonical_worktree_path)?;
        let mut child = Command::new("git")
            .args(["status", "--porcelain=v1", "-z"])
            .current_dir(worktree)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| GitExecutorError::SpawnFailed)?;
        let output = read_bounded(
            child.stdout.take().ok_or(GitExecutorError::SpawnFailed)?,
            &mut child,
        )?;
        if !child
            .wait()
            .map_err(|_| GitExecutorError::StatusFailed)?
            .success()
        {
            return Err(GitExecutorError::StatusFailed);
        }
        let entries = parse_porcelain_v1_z(&output).map_err(|_| GitExecutorError::InvalidOutput)?;
        Ok(GitStatusSummary {
            entry_count: u32::try_from(entries.len()).unwrap_or(u32::MAX),
            output_digest_sha256: hex::encode(Sha256::digest(output)),
        })
    }
}

fn canonical_worktree(value: &str) -> Result<std::path::PathBuf, GitExecutorError> {
    let canonical = Path::new(value)
        .canonicalize()
        .map_err(|_| GitExecutorError::InvalidWorktree)?;
    (canonical == Path::new(value))
        .then_some(canonical)
        .ok_or(GitExecutorError::InvalidWorktree)
}

fn read_bounded(
    mut stdout: impl Read,
    child: &mut std::process::Child,
) -> Result<Vec<u8>, GitExecutorError> {
    let mut output = Vec::new();
    let mut chunk = [0_u8; 8_192];
    loop {
        let Ok(read) = stdout.read(&mut chunk) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(GitExecutorError::StatusFailed);
        };
        if read == 0 {
            return Ok(output);
        }
        if output.len().saturating_add(read) > MAX_STATUS_BYTES {
            let _ = child.kill();
            let _ = child.wait();
            return Err(GitExecutorError::OutputTooLarge);
        }
        output.extend_from_slice(&chunk[..read]);
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, process::Command};

    use gent_ports::{GitExecutor, GitExecutorError, GitStatusOperation};

    use super::SystemGitExecutor;

    fn git(directory: &std::path::Path, args: &[&str]) {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(directory)
                .status()
                .unwrap()
                .success()
        );
    }

    #[test]
    fn reads_only_a_canonical_temporary_worktree() {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "--quiet"]);
        fs::write(directory.path().join("changed.txt"), "change").unwrap();
        let operation = GitStatusOperation {
            canonical_worktree_path: directory
                .path()
                .canonicalize()
                .unwrap()
                .display()
                .to_string(),
        };
        let summary = SystemGitExecutor.status(&operation).unwrap();
        assert_eq!(summary.entry_count, 1);
        assert_eq!(summary.output_digest_sha256.len(), 64);
    }

    #[test]
    fn rejects_noncanonical_or_missing_worktrees() {
        assert_eq!(
            SystemGitExecutor.status(&GitStatusOperation {
                canonical_worktree_path: "/definitely/not/a/gent/worktree".into(),
            }),
            Err(GitExecutorError::InvalidWorktree)
        );
    }
}
