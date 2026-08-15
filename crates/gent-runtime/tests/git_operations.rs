use gent_ports::GitOperationUpdate;
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    CapabilitySet, GitOperationKind, GitOperationPhase, GitOperationRecord, RepositoryRecord,
    WorkspaceRecord, WorktreeRecord,
};

fn coordinator() -> Coordinator<SqliteLedger> {
    Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default())
}

fn prepare(coordinator: &Coordinator<SqliteLedger>) {
    coordinator
        .create_workspace(&WorkspaceRecord {
            workspace_id: "workspace".into(),
            canonical_path: "/workspace".into(),
        })
        .unwrap();
    coordinator
        .create_repository(&RepositoryRecord {
            repository_id: "repository".into(),
            workspace_id: "workspace".into(),
            canonical_path: "/workspace/repository".into(),
        })
        .unwrap();
    coordinator
        .create_worktree(&WorktreeRecord {
            worktree_id: "worktree".into(),
            repository_id: "repository".into(),
            canonical_path: "/workspace/repository/worktree".into(),
        })
        .unwrap();
    coordinator
        .create_run(gent_core::Run {
            id: "run".into(),
            parent_run_id: None,
            provider: "claude".into(),
        })
        .unwrap();
}

#[test]
fn coordinator_enforces_monotonic_git_operation_transitions() {
    let coordinator = coordinator();
    prepare(&coordinator);
    coordinator
        .create_git_operation(&GitOperationRecord {
            operation_id: "operation".into(),
            worktree_id: "worktree".into(),
            run_id: "run".into(),
            kind: GitOperationKind::Status,
            phase: GitOperationPhase::Requested,
        })
        .unwrap();
    coordinator
        .transition_git_operation(
            "operation",
            GitOperationPhase::Requested,
            GitOperationPhase::Running,
        )
        .unwrap();
    assert!(matches!(
        coordinator
            .transition_git_operation(
                "operation",
                GitOperationPhase::Requested,
                GitOperationPhase::Running,
            )
            .unwrap(),
        GitOperationUpdate::Current(_)
    ));
    assert!(
        coordinator
            .transition_git_operation(
                "operation",
                GitOperationPhase::Running,
                GitOperationPhase::Requested
            )
            .is_err()
    );
}
