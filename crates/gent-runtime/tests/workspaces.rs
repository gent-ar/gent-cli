use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{CapabilitySet, RepositoryRecord, WorkspaceRecord, WorktreeRecord};

fn coordinator() -> Coordinator<SqliteLedger> {
    Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default())
}

fn workspace() -> WorkspaceRecord {
    WorkspaceRecord {
        workspace_id: "workspace-a".into(),
        canonical_path: "/workspace".into(),
    }
}

fn repository() -> RepositoryRecord {
    RepositoryRecord {
        repository_id: "repository-a".into(),
        workspace_id: "workspace-a".into(),
        canonical_path: "/workspace/repository".into(),
    }
}

fn worktree() -> WorktreeRecord {
    WorktreeRecord {
        worktree_id: "worktree-a".into(),
        repository_id: "repository-a".into(),
        canonical_path: "/workspace/repository/worktree".into(),
    }
}

#[test]
fn coordinator_persists_and_lists_the_workspace_hierarchy() {
    let coordinator = coordinator();
    coordinator.create_workspace(&workspace()).unwrap();
    coordinator.create_repository(&repository()).unwrap();
    coordinator.create_worktree(&worktree()).unwrap();
    assert_eq!(
        coordinator.workspace_repositories("workspace-a").unwrap(),
        vec![repository()]
    );
    assert_eq!(
        coordinator.repository_worktrees("repository-a").unwrap(),
        vec![worktree()]
    );
}

#[test]
fn coordinator_preserves_parent_invariants() {
    let coordinator = coordinator();
    assert!(coordinator.create_repository(&repository()).is_err());
    coordinator.create_workspace(&workspace()).unwrap();
    assert!(coordinator.create_worktree(&worktree()).is_err());
}
