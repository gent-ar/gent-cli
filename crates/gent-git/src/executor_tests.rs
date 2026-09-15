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
fn repository_root_distinguishes_a_non_repository() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = directory.path().canonicalize().unwrap();
    assert_eq!(
        SystemGitExecutor.repository_root(&canonical.display().to_string()),
        Err(GitExecutorError::NotRepository)
    );
}

#[test]
fn repository_root_resolves_a_linked_worktree_to_its_own_root_not_the_main_repo() {
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
    let worktree_dir = directory.path().join("linked-worktree");
    git(
        directory.path(),
        &[
            "worktree",
            "add",
            "-b",
            "feature/worktree",
            worktree_dir.to_str().unwrap(),
        ],
    );
    let canonical_worktree_dir = worktree_dir.canonicalize().unwrap();
    let canonical_main_dir = directory.path().canonicalize().unwrap();

    let root = SystemGitExecutor
        .repository_root(&canonical_worktree_dir.display().to_string())
        .unwrap();

    assert_eq!(std::path::Path::new(&root), canonical_worktree_dir);
    assert_ne!(std::path::Path::new(&root), canonical_main_dir);
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
        std::path::Path::new(&worktree.canonical_path) == directory.path().canonicalize().unwrap()
    }));
}

#[test]
fn report_includes_commit_branch_stash_and_tracking_state() {
    let directory = tempfile::tempdir().unwrap();
    git(
        directory.path(),
        &["init", "--quiet", "--initial-branch=main"],
    );
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
    assert_eq!(report.stashes[0].message, "On main: save search");
    assert_eq!(report.remote_status.ahead, 0);
    assert_eq!(report.remote_status.behind, 0);
    assert!(report.remote_status.tracking_branch.is_none());
}

#[test]
fn report_for_a_linked_worktree_shows_its_own_branch_and_the_main_repo_path() {
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
    let worktree_dir = directory.path().join("linked-worktree");
    git(
        directory.path(),
        &[
            "worktree",
            "add",
            "-b",
            "feature/worktree",
            worktree_dir.to_str().unwrap(),
        ],
    );
    let canonical_worktree_dir = worktree_dir.canonicalize().unwrap();
    let canonical_main_dir = directory.path().canonicalize().unwrap();

    let report = SystemGitExecutor
        .report(&canonical_worktree_dir.display().to_string())
        .unwrap();

    assert_eq!(report.branch.as_deref(), Some("feature/worktree"));
    assert_eq!(report.worktrees.len(), 2);
    assert!(
        report
            .worktrees
            .iter()
            .any(|worktree| std::path::Path::new(&worktree.canonical_path) == canonical_main_dir)
    );
    assert!(report.worktrees.iter().any(|worktree| {
        std::path::Path::new(&worktree.canonical_path) == canonical_worktree_dir
            && worktree.branch.as_deref() == Some("feature/worktree")
    }));
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
