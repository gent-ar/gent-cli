#[cfg(test)]
#[path = "executor_tests.rs"]
mod executor_tests;
#[path = "report.rs"]
mod report;

use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
};

use gent_ports::{GitExecutor, GitExecutorError, GitReport, GitStatusOperation, GitStatusSummary};
use gent_types::WorkspaceGitFileStatus;
use sha2::{Digest, Sha256};

use crate::parse_porcelain_v1_z;

const MAX_STATUS_BYTES: usize = 1_048_576;

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemGitExecutor;

impl GitExecutor for SystemGitExecutor {
    fn status(&self, operation: &GitStatusOperation) -> Result<GitStatusSummary, GitExecutorError> {
        let worktree = canonical_worktree(&operation.canonical_worktree_path)?;
        let output = git_output(&worktree, &["status", "--porcelain=v1", "-z"])?;
        let entries = parse_porcelain_v1_z(&output).map_err(|_| GitExecutorError::InvalidOutput)?;
        Ok(GitStatusSummary {
            entry_count: u32::try_from(entries.len()).unwrap_or(u32::MAX),
            branch_name: report::branch_name(&worktree)?,
            output_digest_sha256: hex::encode(Sha256::digest(output)),
        })
    }

    fn repository_root(&self, canonical_path: &str) -> Result<String, GitExecutorError> {
        let directory = canonical_worktree(canonical_path)?;
        if !crate::repository_marker::has_repository_marker(&directory) {
            return Err(GitExecutorError::NotRepository);
        }
        let output = git_output(&directory, &["rev-parse", "--show-toplevel"])?;
        let root = String::from_utf8(output).map_err(|_| GitExecutorError::InvalidOutput)?;
        let root = root.trim();
        if root.is_empty() || root.len() > 4096 || root.contains('\0') {
            return Err(GitExecutorError::InvalidOutput);
        }
        Path::new(root)
            .canonicalize()
            .map_err(|_| GitExecutorError::InvalidWorktree)
            .map(|path| path.display().to_string())
    }

    fn report(&self, canonical_repository_root: &str) -> Result<GitReport, GitExecutorError> {
        let root = canonical_worktree(canonical_repository_root)?;
        let output = git_output(&root, &["status", "--porcelain=v1", "-z"])?;
        let entries = parse_porcelain_v1_z(&output).map_err(|_| GitExecutorError::InvalidOutput)?;
        let files = entries
            .into_iter()
            .map(|entry| WorkspaceGitFileStatus {
                index_status: entry.index_status,
                worktree_status: entry.worktree_status,
                path: entry.path,
                original_path: entry.original_path,
            })
            .collect();
        let branches = report::branches(&root)?;
        Ok(GitReport {
            branch: report::branch_name(&root)?,
            files,
            worktrees: report::list_worktrees(&root)?,
            recent_commits: report::recent_commits(&root)?,
            remote_status: report::remote_status(&branches),
            branches,
            stashes: report::stashes(&root)?,
        })
    }

    fn checkout_paths(
        &self,
        canonical_repository_root: &str,
        paths: &[String],
    ) -> Result<(), GitExecutorError> {
        let root = canonical_worktree(canonical_repository_root)?;
        if paths.is_empty() || paths.iter().any(|path| path.contains('\0')) {
            return Err(GitExecutorError::InvalidOutput);
        }
        let mut command = Command::new("git");
        command
            .arg("checkout")
            .arg("--")
            .args(paths)
            .current_dir(&root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let status = command
            .status()
            .map_err(|_| GitExecutorError::SpawnFailed)?;
        status
            .success()
            .then_some(())
            .ok_or(GitExecutorError::StatusFailed)
    }
}

pub(crate) fn git_output(root: &Path, args: &[&str]) -> Result<Vec<u8>, GitExecutorError> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| GitExecutorError::SpawnFailed)?;
    let output = read_bounded(
        child.stdout.take().ok_or(GitExecutorError::SpawnFailed)?,
        &mut child,
    )?;
    child
        .wait()
        .map_err(|_| GitExecutorError::StatusFailed)?
        .success()
        .then_some(output)
        .ok_or(GitExecutorError::StatusFailed)
}

pub(crate) fn git_text(root: &Path, args: &[&str]) -> Result<String, GitExecutorError> {
    String::from_utf8(git_output(root, args)?).map_err(|_| GitExecutorError::InvalidOutput)
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
