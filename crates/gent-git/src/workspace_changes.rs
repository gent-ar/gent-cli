use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use gent_ports::GitExecutorError;
use gent_types::WorkspaceFileChange;

use crate::executor::{git_output, git_output_with_index};

pub const MAX_CHANGED_FILES: usize = 500;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceChanges {
    pub files: Vec<WorkspaceFileChange>,
    pub truncated: bool,
}

struct ScratchIndex(PathBuf);

impl Drop for ScratchIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub fn snapshot_tree(workspace: &Path) -> Result<String, GitExecutorError> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let index = text(git_output(
        workspace,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    )?)?;
    let scratch = ScratchIndex(std::env::temp_dir().join(format!(
        "gent-workspace-index-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    match std::fs::copy(&index, &scratch.0) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(GitExecutorError::StatusFailed),
    }
    git_output_with_index(workspace, &["add", "--all", "--", ":/"], Some(&scratch.0))?;
    text(git_output_with_index(
        workspace,
        &["write-tree"],
        Some(&scratch.0),
    )?)
}

pub fn changes_since(
    workspace: &Path,
    baseline: &str,
) -> Result<WorkspaceChanges, GitExecutorError> {
    if !is_object_id(baseline) {
        return Err(GitExecutorError::InvalidOutput);
    }
    let current = snapshot_tree(workspace)?;
    let output = git_output(
        workspace,
        &[
            "diff",
            "--numstat",
            "-z",
            "--no-renames",
            "--relative",
            baseline,
            &current,
        ],
    )?;
    parse_numstat_z(&output)
}

fn parse_numstat_z(output: &[u8]) -> Result<WorkspaceChanges, GitExecutorError> {
    let mut files = Vec::new();
    let mut truncated = false;
    for record in output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let record = std::str::from_utf8(record).map_err(|_| GitExecutorError::InvalidOutput)?;
        let mut fields = record.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) =
            (fields.next(), fields.next(), fields.next())
        else {
            return Err(GitExecutorError::InvalidOutput);
        };
        if added == "-" && removed == "-" {
            continue;
        }
        if files.len() == MAX_CHANGED_FILES {
            truncated = true;
            break;
        }
        files.push(WorkspaceFileChange {
            path: path.to_owned(),
            added: added.parse().map_err(|_| GitExecutorError::InvalidOutput)?,
            removed: removed
                .parse()
                .map_err(|_| GitExecutorError::InvalidOutput)?,
        });
    }
    Ok(WorkspaceChanges { files, truncated })
}

fn text(output: Vec<u8>) -> Result<String, GitExecutorError> {
    let value = String::from_utf8(output).map_err(|_| GitExecutorError::InvalidOutput)?;
    let value = value.trim();
    (!value.is_empty() && !value.contains('\0'))
        .then(|| value.to_owned())
        .ok_or(GitExecutorError::InvalidOutput)
}

fn is_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
#[path = "workspace_changes_tests.rs"]
mod tests;
