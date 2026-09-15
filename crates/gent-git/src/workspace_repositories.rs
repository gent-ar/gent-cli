use std::path::{Path, PathBuf};

use gent_ports::GitExecutorError;

use crate::executor::git_text;

const MAX_DEPTH: usize = 3;
const MAX_DIRECTORIES: usize = 10_000;

pub fn workspace_repositories(workspace: &Path) -> Result<Vec<String>, GitExecutorError> {
    let workspace = workspace
        .canonicalize()
        .map_err(|_| GitExecutorError::InvalidWorktree)?;
    let own = repository(&workspace).map(|found| found.common_dir);
    let mut nested = Vec::new();
    let mut pending = vec![(workspace.clone(), 0)];
    let mut visited = 0;
    while let Some((directory, depth)) = pending.pop() {
        for child in child_directories(&directory) {
            visited += 1;
            if visited > MAX_DIRECTORIES {
                return Ok(presented(&workspace, own.is_some(), nested));
            }
            if child.join(".git").exists()
                && let Some(found) = repository(&child)
                && found.top_level == child
                && Some(&found.common_dir) != own.as_ref()
            {
                nested.push(child.clone());
            }
            if depth + 1 < MAX_DEPTH {
                pending.push((child, depth + 1));
            }
        }
    }
    Ok(presented(&workspace, own.is_some(), nested))
}

struct Repository {
    top_level: PathBuf,
    common_dir: PathBuf,
}

fn repository(directory: &Path) -> Option<Repository> {
    let output = git_text(
        directory,
        &["rev-parse", "--show-toplevel", "--git-common-dir"],
    )
    .ok()?;
    let mut lines = output.lines();
    let top_level = Path::new(lines.next()?).canonicalize().ok()?;
    let common_dir = directory.join(lines.next()?).canonicalize().ok()?;
    Some(Repository {
        top_level,
        common_dir,
    })
}

fn child_directories(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut children = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            !entry.file_name().to_string_lossy().starts_with('.')
                && entry.file_type().is_ok_and(|kind| kind.is_dir())
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    children.sort_by(|left, right| right.cmp(left));
    children
}

fn presented(
    workspace: &Path,
    workspace_is_repository: bool,
    mut nested: Vec<PathBuf>,
) -> Vec<String> {
    if nested.is_empty() {
        return Vec::new();
    }
    nested.sort();
    workspace_is_repository
        .then(|| workspace.to_path_buf())
        .into_iter()
        .chain(nested)
        .map(|path| path.display().to_string())
        .collect()
}

#[cfg(test)]
#[path = "workspace_repositories_tests.rs"]
mod tests;
