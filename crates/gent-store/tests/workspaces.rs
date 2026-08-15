use gent_ports::WorkspaceLedger;
use gent_store::SqliteLedger;
use gent_types::{RepositoryRecord, WorkspaceRecord, WorktreeRecord};

fn workspace() -> WorkspaceRecord {
    WorkspaceRecord {
        workspace_id: "workspace-a".into(),
        canonical_path: "/tmp/workspace".into(),
    }
}

fn repository() -> RepositoryRecord {
    RepositoryRecord {
        repository_id: "repository-a".into(),
        workspace_id: "workspace-a".into(),
        canonical_path: "/tmp/workspace/repository".into(),
    }
}

fn worktree() -> WorktreeRecord {
    WorktreeRecord {
        worktree_id: "worktree-a".into(),
        repository_id: "repository-a".into(),
        canonical_path: "/tmp/workspace/repository/wt".into(),
    }
}

#[test]
fn workspace_hierarchy_survives_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    ledger.create_workspace(&workspace()).unwrap();
    ledger.create_repository(&repository()).unwrap();
    ledger.create_worktree(&worktree()).unwrap();
    drop(ledger);

    let restarted = SqliteLedger::open(path).unwrap();
    assert_eq!(
        restarted.find_workspace("workspace-a").unwrap(),
        Some(workspace())
    );
    assert_eq!(
        restarted
            .list_workspace_repositories("workspace-a")
            .unwrap(),
        vec![repository()]
    );
    assert_eq!(
        restarted.list_repository_worktrees("repository-a").unwrap(),
        vec![worktree()]
    );
}

#[test]
fn hierarchy_rejects_missing_parents_and_invalid_identities() {
    let ledger = SqliteLedger::in_memory().unwrap();
    assert!(ledger.create_repository(&repository()).is_err());
    assert!(
        ledger
            .create_workspace(&WorkspaceRecord {
                workspace_id: String::new(),
                ..workspace()
            })
            .is_err()
    );
    ledger.create_workspace(&workspace()).unwrap();
    ledger.create_repository(&repository()).unwrap();
    assert!(
        ledger
            .create_worktree(&WorktreeRecord {
                canonical_path: String::new(),
                ..worktree()
            })
            .is_err()
    );
}
