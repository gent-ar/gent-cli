#[cfg(test)]
#[path = "executor_tests.rs"]
mod executor_tests;
#[path = "report.rs"]
mod report;

use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

use gent_ports::{GitExecutor, GitExecutorError, GitReport, GitStatusOperation, GitStatusSummary};
use gent_types::WorkspaceGitFileStatus;
use sha2::{Digest, Sha256};

use crate::parse_porcelain_v1_z;

const MAX_STATUS_BYTES: usize = 1_048_576;
const GIT_DEADLINE: Duration = Duration::from_secs(20);

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
        let root = root.as_path();
        std::thread::scope(|scope| {
            let files = scope.spawn(|| status_files(root));
            let branch = scope.spawn(|| report::branch_name(root));
            let branches = scope.spawn(|| report::branches(root));
            let worktrees = scope.spawn(|| report::list_worktrees(root));
            let recent_commits = scope.spawn(|| report::recent_commits(root));
            let stashes = scope.spawn(|| report::stashes(root));
            let branches = joined(branches)?;
            Ok(GitReport {
                branch: joined(branch)?,
                files: joined(files)?,
                worktrees: joined(worktrees)?,
                recent_commits: joined(recent_commits)?,
                remote_status: report::remote_status(&branches),
                branches,
                stashes: joined(stashes)?,
            })
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
        let mut command = gent_types::hidden_command("git");
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

fn status_files(root: &Path) -> Result<Vec<WorkspaceGitFileStatus>, GitExecutorError> {
    let output = git_output(root, &["status", "--porcelain=v1", "-z"])?;
    let entries = parse_porcelain_v1_z(&output).map_err(|_| GitExecutorError::InvalidOutput)?;
    Ok(entries
        .into_iter()
        .map(|entry| WorkspaceGitFileStatus {
            index_status: entry.index_status,
            worktree_status: entry.worktree_status,
            path: entry.path,
            original_path: entry.original_path,
        })
        .collect())
}

fn joined<T>(
    handle: std::thread::ScopedJoinHandle<'_, Result<T, GitExecutorError>>,
) -> Result<T, GitExecutorError> {
    handle.join().unwrap_or(Err(GitExecutorError::StatusFailed))
}

pub(crate) fn git_output(root: &Path, args: &[&str]) -> Result<Vec<u8>, GitExecutorError> {
    bounded_output(
        gent_types::hidden_command("git")
            .arg("--no-optional-locks")
            .args(args)
            .current_dir(root),
        GIT_DEADLINE,
    )
}

fn bounded_output(command: &mut Command, deadline: Duration) -> Result<Vec<u8>, GitExecutorError> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| GitExecutorError::SpawnFailed)?;
    let stdout = child.stdout.take().ok_or(GitExecutorError::SpawnFailed)?;
    let (sender, finished) = mpsc::channel();
    std::thread::spawn(move || sender.send(read_bounded(stdout)));
    let output = finished
        .recv_timeout(deadline)
        .unwrap_or(Err(GitExecutorError::StatusFailed));
    if output.is_err() {
        let _ = child.kill();
    }
    let succeeded = child
        .wait()
        .map_err(|_| GitExecutorError::StatusFailed)?
        .success();
    let output = output?;
    succeeded
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

fn read_bounded(mut stdout: impl Read) -> Result<Vec<u8>, GitExecutorError> {
    let mut output = Vec::new();
    let mut chunk = [0_u8; 8_192];
    loop {
        let Ok(read) = stdout.read(&mut chunk) else {
            return Err(GitExecutorError::StatusFailed);
        };
        if read == 0 {
            return Ok(output);
        }
        if output.len().saturating_add(read) > MAX_STATUS_BYTES {
            return Err(GitExecutorError::OutputTooLarge);
        }
        output.extend_from_slice(&chunk[..read]);
    }
}
