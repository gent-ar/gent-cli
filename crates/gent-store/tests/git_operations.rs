use gent_ports::{GitOperationLedger, GitOperationUpdate, Ledger, RunRecord, WorkspaceLedger};
use gent_store::SqliteLedger;
use gent_types::{
    GitOperationKind, GitOperationPhase, GitOperationRecord, RepositoryRecord, WorkspaceRecord,
    WorktreeRecord,
};

fn operation(phase: GitOperationPhase) -> GitOperationRecord {
    GitOperationRecord {
        operation_id: "operation-a".into(),
        worktree_id: "worktree-a".into(),
        run_id: "run-a".into(),
        kind: GitOperationKind::Status,
        phase,
    }
}

fn prepare(ledger: &SqliteLedger) {
    ledger
        .create_workspace(&WorkspaceRecord {
            workspace_id: "workspace-a".into(),
            canonical_path: "/workspace".into(),
        })
        .unwrap();
    ledger
        .create_repository(&RepositoryRecord {
            repository_id: "repository-a".into(),
            workspace_id: "workspace-a".into(),
            canonical_path: "/workspace/repository".into(),
        })
        .unwrap();
    ledger
        .create_worktree(&WorktreeRecord {
            worktree_id: "worktree-a".into(),
            repository_id: "repository-a".into(),
            canonical_path: "/workspace/repository/worktree".into(),
        })
        .unwrap();
    ledger
        .create_run(&RunRecord {
            run_id: "run-a".into(),
            parent_run_id: None,
            provider: "claude".into(),
        })
        .unwrap();
}

#[test]
fn git_operation_is_durable_and_optimistic() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    prepare(&ledger);
    ledger
        .create_git_operation(&operation(GitOperationPhase::Requested))
        .unwrap();
    assert!(matches!(
        ledger
            .replace_git_operation_phase(
                "operation-a",
                GitOperationPhase::Requested,
                GitOperationPhase::Running
            )
            .unwrap(),
        GitOperationUpdate::Applied(GitOperationRecord {
            phase: GitOperationPhase::Running,
            ..
        })
    ));
    assert!(matches!(
        ledger
            .replace_git_operation_phase(
                "operation-a",
                GitOperationPhase::Requested,
                GitOperationPhase::Failed
            )
            .unwrap(),
        GitOperationUpdate::Current(_)
    ));
    drop(ledger);
    assert_eq!(
        SqliteLedger::open(path)
            .unwrap()
            .find_git_operation("operation-a")
            .unwrap(),
        Some(operation(GitOperationPhase::Running))
    );
}
