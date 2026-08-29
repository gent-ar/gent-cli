//! Fixed-argv, bounded operating-system edge for read-only Git status.

use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
};

use gent_ports::{GitExecutor, GitExecutorError, GitReport, GitStatusOperation, GitStatusSummary};
use gent_types::{
    WorkspaceGitBranch, WorkspaceGitCommit, WorkspaceGitFileStatus, WorkspaceGitRemoteStatus,
    WorkspaceGitStashEntry, WorkspaceGitWorktree,
};
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
            .current_dir(&worktree)
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
            branch_name: branch_name(&worktree)?,
            output_digest_sha256: hex::encode(Sha256::digest(output)),
        })
    }

    fn repository_root(&self, canonical_path: &str) -> Result<String, GitExecutorError> {
        let directory = canonical_worktree(canonical_path)?;
        let mut child = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(&directory)
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
        let mut child = Command::new("git")
            .args(["status", "--porcelain=v1", "-z"])
            .current_dir(&root)
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
        let files = entries
            .into_iter()
            .map(|entry| WorkspaceGitFileStatus {
                index_status: entry.index_status,
                worktree_status: entry.worktree_status,
                path: entry.path,
                original_path: entry.original_path,
            })
            .collect();
        let branches = branches(&root)?;
        Ok(GitReport {
            branch: branch_name(&root)?,
            files,
            worktrees: list_worktrees(&root)?,
            recent_commits: recent_commits(&root)?,
            remote_status: remote_status(&branches),
            branches,
            stashes: stashes(&root)?,
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

fn git_output(root: &Path, args: &[&str]) -> Result<Vec<u8>, GitExecutorError> {
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

fn git_text(root: &Path, args: &[&str]) -> Result<String, GitExecutorError> {
    String::from_utf8(git_output(root, args)?).map_err(|_| GitExecutorError::InvalidOutput)
}

fn recent_commits(root: &Path) -> Result<Vec<WorkspaceGitCommit>, GitExecutorError> {
    let records = match git_text(root, &["log", "--format=%H%x00%s%x00%an%x00%ai", "-20"]) {
        Ok(records) => records,
        Err(GitExecutorError::StatusFailed) => return Ok(vec![]),
        Err(error) => return Err(error),
    };
    records
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let fields = line.split('\0').collect::<Vec<_>>();
            (fields.len() == 4)
                .then(|| WorkspaceGitCommit {
                    hash: fields[0].to_owned(),
                    message: fields[1].to_owned(),
                    author: fields[2].to_owned(),
                    date: fields[3].to_owned(),
                })
                .ok_or(GitExecutorError::InvalidOutput)
        })
        .collect()
}

fn branches(root: &Path) -> Result<Vec<WorkspaceGitBranch>, GitExecutorError> {
    let records = git_text(
        root,
        &[
            "for-each-ref",
            "--format=%(refname:short)%09%(HEAD)%09%(upstream:short)%09%(upstream:track)",
            "refs/heads/",
        ],
    )?;
    records
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let fields = line.split('\t').collect::<Vec<_>>();
            (fields.len() == 4)
                .then(|| WorkspaceGitBranch {
                    name: fields[0].to_owned(),
                    is_current: fields[1].trim() == "*",
                    tracking_remote: (!fields[2].is_empty()).then(|| fields[2].to_owned()),
                    ahead: tracking_count(fields[3], "ahead"),
                    behind: tracking_count(fields[3], "behind"),
                })
                .ok_or(GitExecutorError::InvalidOutput)
        })
        .collect()
}

fn stashes(root: &Path) -> Result<Vec<WorkspaceGitStashEntry>, GitExecutorError> {
    let records = git_text(root, &["stash", "list", "--format=%gd%x00%gs"])?;
    records
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let fields = line.split('\0').collect::<Vec<_>>();
            if fields.len() != 2 {
                return Err(GitExecutorError::InvalidOutput);
            }
            let index = fields[0]
                .strip_prefix("stash@{")
                .and_then(|value| value.strip_suffix('}'))
                .and_then(|value| value.parse().ok())
                .ok_or(GitExecutorError::InvalidOutput)?;
            Ok(WorkspaceGitStashEntry {
                index,
                message: fields[1].to_owned(),
            })
        })
        .collect()
}

fn remote_status(branches: &[WorkspaceGitBranch]) -> WorkspaceGitRemoteStatus {
    let branch = branches.iter().find(|branch| branch.is_current);
    WorkspaceGitRemoteStatus {
        ahead: branch.as_ref().map_or(0, |branch| branch.ahead),
        behind: branch.as_ref().map_or(0, |branch| branch.behind),
        tracking_branch: branch.and_then(|branch| branch.tracking_remote.clone()),
    }
}

fn tracking_count(value: &str, label: &str) -> u32 {
    value
        .split(['[', ']', ',', ' '])
        .collect::<Vec<_>>()
        .windows(2)
        .find_map(|fields| {
            (fields[0] == label)
                .then(|| fields[1].parse().ok())
                .flatten()
        })
        .unwrap_or(0)
}

fn list_worktrees(root: &Path) -> Result<Vec<WorkspaceGitWorktree>, GitExecutorError> {
    let mut child = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
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
    if !child
        .wait()
        .map_err(|_| GitExecutorError::StatusFailed)?
        .success()
    {
        return Err(GitExecutorError::StatusFailed);
    }
    let text = String::from_utf8(output).map_err(|_| GitExecutorError::InvalidOutput)?;
    parse_worktree_list(&text)
}

fn parse_worktree_list(text: &str) -> Result<Vec<WorkspaceGitWorktree>, GitExecutorError> {
    let mut worktrees = Vec::new();
    let mut canonical_path: Option<String> = None;
    let mut head: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut is_detached = false;
    let mut is_locked = false;
    let flush = |worktrees: &mut Vec<WorkspaceGitWorktree>,
                 canonical_path: &mut Option<String>,
                 head: &mut Option<String>,
                 branch: &mut Option<String>,
                 is_detached: &mut bool,
                 is_locked: &mut bool| {
        if let Some(canonical_path) = canonical_path.take() {
            worktrees.push(WorkspaceGitWorktree {
                canonical_path,
                branch: branch.take(),
                head: head.take(),
                is_detached: *is_detached,
                is_locked: *is_locked,
            });
        }
        *is_detached = false;
        *is_locked = false;
    };
    for line in text.lines() {
        if line.is_empty() {
            flush(
                &mut worktrees,
                &mut canonical_path,
                &mut head,
                &mut branch,
                &mut is_detached,
                &mut is_locked,
            );
            continue;
        }
        if let Some(value) = line.strip_prefix("worktree ") {
            flush(
                &mut worktrees,
                &mut canonical_path,
                &mut head,
                &mut branch,
                &mut is_detached,
                &mut is_locked,
            );
            canonical_path = Some(value.to_owned());
        } else if let Some(value) = line.strip_prefix("HEAD ") {
            head = Some(value.to_owned());
        } else if let Some(value) = line.strip_prefix("branch ") {
            branch = Some(
                value
                    .strip_prefix("refs/heads/")
                    .unwrap_or(value)
                    .to_owned(),
            );
        } else if line == "detached" {
            is_detached = true;
        } else if line.starts_with("locked") {
            is_locked = true;
        }
    }
    flush(
        &mut worktrees,
        &mut canonical_path,
        &mut head,
        &mut branch,
        &mut is_detached,
        &mut is_locked,
    );
    Ok(worktrees)
}

fn branch_name(worktree: &Path) -> Result<Option<String>, GitExecutorError> {
    let mut child = Command::new("git")
        .args(["branch", "--show-current"])
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
    let branch = String::from_utf8(output).map_err(|_| GitExecutorError::InvalidOutput)?;
    let branch = branch.trim();
    if branch.is_empty() {
        return Ok(None);
    }
    (branch.len() <= 256 && !branch.contains('\0'))
        .then(|| Some(branch.to_owned()))
        .ok_or(GitExecutorError::InvalidOutput)
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
        assert!(summary.branch_name.is_some());
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

    #[test]
    fn repository_root_resolves_from_a_subdirectory() {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "--quiet"]);
        let canonical_root = directory.path().canonicalize().unwrap();
        let subdirectory = canonical_root.join("nested");
        fs::create_dir(&subdirectory).unwrap();
        let root = SystemGitExecutor
            .repository_root(&subdirectory.display().to_string())
            .unwrap();
        assert_eq!(std::path::Path::new(&root), canonical_root);
    }

    #[test]
    fn report_wires_a_pending_rename_through_to_the_client() {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "--quiet"]);
        git(
            directory.path(),
            &["config", "user.email", "test@example.com"],
        );
        git(directory.path(), &["config", "user.name", "Test"]);
        fs::write(directory.path().join("a.txt"), "content").unwrap();
        git(directory.path(), &["add", "a.txt"]);
        git(directory.path(), &["commit", "--quiet", "-m", "add a"]);
        git(directory.path(), &["mv", "a.txt", "b.txt"]);
        let root = directory
            .path()
            .canonicalize()
            .unwrap()
            .display()
            .to_string();
        let report = SystemGitExecutor.report(&root).unwrap();
        assert!(report.branch.is_some());
        assert_eq!(report.files.len(), 1);
        assert_eq!(report.files[0].path, "b.txt");
        assert_eq!(report.files[0].original_path.as_deref(), Some("a.txt"));
        assert!(report.worktrees.iter().any(|worktree| {
            std::path::Path::new(&worktree.canonical_path)
                == directory.path().canonicalize().unwrap()
        }));
    }

    #[test]
    fn report_includes_commit_branch_stash_and_tracking_state() {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "--quiet"]);
        git(
            directory.path(),
            &["config", "user.email", "test@example.com"],
        );
        git(directory.path(), &["config", "user.name", "Test"]);
        fs::write(directory.path().join("a.txt"), "content").unwrap();
        git(directory.path(), &["add", "a.txt"]);
        git(directory.path(), &["commit", "--quiet", "-m", "add a"]);
        git(directory.path(), &["branch", "feature/search"]);
        fs::write(directory.path().join("a.txt"), "changed").unwrap();
        git(
            directory.path(),
            &["stash", "push", "--quiet", "-m", "save search"],
        );
        let root = directory
            .path()
            .canonicalize()
            .unwrap()
            .display()
            .to_string();
        let report = SystemGitExecutor.report(&root).unwrap();
        assert_eq!(report.recent_commits.len(), 1);
        assert_eq!(report.recent_commits[0].message, "add a");
        assert!(
            report
                .branches
                .iter()
                .any(|branch| branch.name == "feature/search")
        );
        assert_eq!(report.stashes.len(), 1);
        assert_eq!(report.stashes[0].message, "On master: save search");
        assert_eq!(report.remote_status.ahead, 0);
        assert_eq!(report.remote_status.behind, 0);
        assert!(report.remote_status.tracking_branch.is_none());
    }

    #[test]
    fn checkout_paths_restores_tracked_content() {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "--quiet"]);
        git(
            directory.path(),
            &["config", "user.email", "test@example.com"],
        );
        git(directory.path(), &["config", "user.name", "Test"]);
        fs::write(directory.path().join("a.txt"), "original").unwrap();
        git(directory.path(), &["add", "a.txt"]);
        git(directory.path(), &["commit", "--quiet", "-m", "add a"]);
        fs::write(directory.path().join("a.txt"), "modified").unwrap();
        let root = directory
            .path()
            .canonicalize()
            .unwrap()
            .display()
            .to_string();
        SystemGitExecutor
            .checkout_paths(&root, &["a.txt".to_owned()])
            .unwrap();
        assert_eq!(
            fs::read_to_string(directory.path().join("a.txt")).unwrap(),
            "original"
        );
    }
}
