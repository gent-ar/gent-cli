use std::path::Path;

use gent_ports::GitExecutorError;
use gent_types::{
    WorkspaceGitBranch, WorkspaceGitCommit, WorkspaceGitRemoteStatus, WorkspaceGitStashEntry,
    WorkspaceGitWorktree,
};

use super::{git_output, git_text};

pub(crate) fn recent_commits(root: &Path) -> Result<Vec<WorkspaceGitCommit>, GitExecutorError> {
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

pub(crate) fn branches(root: &Path) -> Result<Vec<WorkspaceGitBranch>, GitExecutorError> {
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

pub(crate) fn stashes(root: &Path) -> Result<Vec<WorkspaceGitStashEntry>, GitExecutorError> {
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

pub(crate) fn remote_status(branches: &[WorkspaceGitBranch]) -> WorkspaceGitRemoteStatus {
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

pub(crate) fn list_worktrees(root: &Path) -> Result<Vec<WorkspaceGitWorktree>, GitExecutorError> {
    let output = git_output(root, &["worktree", "list", "--porcelain"])?;
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

pub(crate) fn branch_name(worktree: &Path) -> Result<Option<String>, GitExecutorError> {
    let output = git_output(worktree, &["branch", "--show-current"])?;
    let branch = String::from_utf8(output).map_err(|_| GitExecutorError::InvalidOutput)?;
    let branch = branch.trim();
    if branch.is_empty() {
        return Ok(None);
    }
    (branch.len() <= 256 && !branch.contains('\0'))
        .then(|| Some(branch.to_owned()))
        .ok_or(GitExecutorError::InvalidOutput)
}
