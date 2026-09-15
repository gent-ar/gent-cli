use gent_protocol::WorkspaceGitFrame;
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{CapabilitySet, WorkspaceRecord};
use std::process::Command;

use crate::workspace_git_api::exchange;

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
fn status_reads_a_registered_workspaces_repository() {
    let directory = tempfile::tempdir().unwrap();
    git(directory.path(), &["init", "--quiet"]);
    let canonical_path = directory
        .path()
        .canonicalize()
        .unwrap()
        .display()
        .to_string();
    let coordinator =
        Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
    coordinator
        .create_workspace(&WorkspaceRecord {
            workspace_id: "workspace-1".into(),
            canonical_path,
        })
        .unwrap();
    let reply = exchange(
        &coordinator,
        WorkspaceGitFrame::StatusRequest {
            request_id: "request-1".into(),
            workspace_id: "workspace-1".into(),
        },
    )
    .unwrap();
    let WorkspaceGitFrame::Status { report, .. } = reply else {
        unreachable!()
    };
    assert!(report.is_some());
}

#[test]
fn status_omits_the_report_when_the_workspace_is_not_a_git_repository() {
    let directory = tempfile::tempdir().unwrap();
    let canonical_path = directory
        .path()
        .canonicalize()
        .unwrap()
        .display()
        .to_string();
    let coordinator =
        Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
    coordinator
        .create_workspace(&WorkspaceRecord {
            workspace_id: "workspace-1".into(),
            canonical_path,
        })
        .unwrap();
    let reply = exchange(
        &coordinator,
        WorkspaceGitFrame::StatusRequest {
            request_id: "request-1".into(),
            workspace_id: "workspace-1".into(),
        },
    )
    .unwrap();
    let WorkspaceGitFrame::Status { report, .. } = reply else {
        unreachable!()
    };
    assert!(report.is_none());
}

#[test]
fn status_rejects_a_broken_repository_instead_of_reporting_non_git() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join(".git")).unwrap();
    let canonical_path = directory
        .path()
        .canonicalize()
        .unwrap()
        .display()
        .to_string();
    let coordinator =
        Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
    coordinator
        .create_workspace(&WorkspaceRecord {
            workspace_id: "workspace-1".into(),
            canonical_path,
        })
        .unwrap();
    let result = exchange(
        &coordinator,
        WorkspaceGitFrame::StatusRequest {
            request_id: "request-1".into(),
            workspace_id: "workspace-1".into(),
        },
    );
    assert!(result.is_err());
}

#[test]
fn sub_repos_lists_nested_repositories() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("repo-a")).unwrap();
    git(&directory.path().join("repo-a"), &["init", "--quiet"]);
    let canonical_path = directory
        .path()
        .canonicalize()
        .unwrap()
        .display()
        .to_string();
    let coordinator =
        Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
    coordinator
        .create_workspace(&WorkspaceRecord {
            workspace_id: "workspace-1".into(),
            canonical_path,
        })
        .unwrap();
    let reply = exchange(
        &coordinator,
        WorkspaceGitFrame::SubReposRequest {
            request_id: "request-1".into(),
            workspace_id: "workspace-1".into(),
        },
    )
    .unwrap();
    let WorkspaceGitFrame::SubRepos {
        canonical_paths, ..
    } = reply
    else {
        unreachable!()
    };
    assert_eq!(canonical_paths.len(), 1);
    assert!(canonical_paths[0].ends_with("repo-a"));
}

#[test]
fn status_for_a_linked_worktree_reports_its_own_branch_and_the_main_repo_path() {
    let directory = tempfile::tempdir().unwrap();
    git(directory.path(), &["init", "--quiet"]);
    git(
        directory.path(),
        &["config", "user.email", "test@example.com"],
    );
    git(directory.path(), &["config", "user.name", "Test"]);
    std::fs::write(directory.path().join("a.txt"), "content").unwrap();
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
    let canonical_worktree_path = worktree_dir.canonicalize().unwrap().display().to_string();
    let canonical_main_path = directory
        .path()
        .canonicalize()
        .unwrap()
        .display()
        .to_string();

    let coordinator =
        Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
    coordinator
        .create_workspace(&WorkspaceRecord {
            workspace_id: "workspace-1".into(),
            canonical_path: canonical_worktree_path.clone(),
        })
        .unwrap();
    let reply = exchange(
        &coordinator,
        WorkspaceGitFrame::StatusRequest {
            request_id: "request-1".into(),
            workspace_id: "workspace-1".into(),
        },
    )
    .unwrap();
    let WorkspaceGitFrame::Status { report, .. } = reply else {
        unreachable!()
    };
    let report = report.unwrap();

    assert_eq!(report.repository_root, canonical_worktree_path);
    assert_eq!(report.branch.as_deref(), Some("feature/worktree"));
    assert_eq!(report.worktrees.len(), 2);
    assert!(
        report
            .worktrees
            .iter()
            .any(|worktree| worktree.canonical_path == canonical_main_path)
    );
    assert!(
        report.worktrees.iter().any(
            |worktree| worktree.canonical_path == canonical_worktree_path
                && worktree.branch.as_deref() == Some("feature/worktree")
        )
    );
}

#[test]
fn status_rejects_an_unknown_workspace() {
    let coordinator =
        Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
    let result = exchange(
        &coordinator,
        WorkspaceGitFrame::StatusRequest {
            request_id: "request-1".into(),
            workspace_id: "missing".into(),
        },
    );
    assert!(result.is_err());
}

#[test]
fn an_unknown_workspace_is_a_typed_not_found_rejection() {
    let coordinator =
        Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
    for frame in [
        WorkspaceGitFrame::StatusRequest {
            request_id: "request-1".into(),
            workspace_id: "workspace-1".into(),
        },
        WorkspaceGitFrame::SubReposRequest {
            request_id: "request-2".into(),
            workspace_id: "workspace-1".into(),
        },
    ] {
        assert_eq!(
            exchange(&coordinator, frame).unwrap_err().code,
            "workspaceNotFound"
        );
    }
}
