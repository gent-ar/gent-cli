use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use gent_ports::{
    GitExecutor, GitExecutorError, GitOperationLedger, GitStatusOperation, GitStatusSummary,
    Ledger, ReceiptClaim, RunRecord, WorkspaceLedger, WorktreeLease,
};
use gent_runtime::{GitStatusAuthority, GitStatusRequest, GitStatusService, GitStatusState};
use gent_store::SqliteLedger;
use gent_types::{
    Command, Event, GitOperationKind, GitOperationPhase, GitOperationRecord, HostEpoch, ReceiptId,
    RepositoryRecord, WorkspaceRecord, WorktreeRecord,
};
use serde_json::json;

#[derive(Clone, Debug, Default)]
struct FakeGit {
    calls: Arc<AtomicUsize>,
    paths: Arc<Mutex<Vec<String>>>,
}

impl GitExecutor for FakeGit {
    fn status(&self, operation: &GitStatusOperation) -> Result<GitStatusSummary, GitExecutorError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.paths
            .lock()
            .unwrap()
            .push(operation.canonical_worktree_path.clone());
        Ok(GitStatusSummary {
            entry_count: 2,
            output_digest_sha256: "a".repeat(64),
        })
    }
}

fn request(key: &str, operation_id: &str) -> GitStatusRequest {
    GitStatusRequest {
        receipt_id: ReceiptId(format!("receipt-{key}")),
        idempotency_key: key.into(),
        host_epoch: HostEpoch(1),
        operation: GitOperationRecord {
            operation_id: operation_id.into(),
            worktree_id: "worktree".into(),
            run_id: "run".into(),
            kind: GitOperationKind::Status,
            phase: GitOperationPhase::Requested,
        },
        lease: WorktreeLease {
            worktree_id: "worktree".into(),
            run_id: "run".into(),
            lease_token: format!("lease-{key}"),
            host_epoch: HostEpoch(1),
        },
    }
}

fn ledger_with_run() -> SqliteLedger {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_run(&RunRecord {
            run_id: "run".into(),
            parent_run_id: None,
            provider: "test".into(),
        })
        .unwrap();
    ledger
        .create_workspace(&WorkspaceRecord {
            workspace_id: "workspace".into(),
            canonical_path: "/canonical".into(),
        })
        .unwrap();
    ledger
        .create_repository(&RepositoryRecord {
            repository_id: "repository".into(),
            workspace_id: "workspace".into(),
            canonical_path: "/canonical/repository".into(),
        })
        .unwrap();
    ledger
        .create_worktree(&WorktreeRecord {
            worktree_id: "worktree".into(),
            repository_id: "repository".into(),
            canonical_path: "/canonical/worktree".into(),
        })
        .unwrap();
    ledger
}

#[test]
fn observer_mode_never_claims_a_receipt_lease_or_git_effect() {
    let ledger = ledger_with_run();
    let fake = FakeGit::default();
    let result = GitStatusService::new(ledger.clone(), fake.clone(), GitStatusAuthority::Observer)
        .execute(&request("observer", "operation-observer"))
        .unwrap();
    assert_eq!(result.state, GitStatusState::DeniedObserver);
    assert!(result.receipt.is_none());
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    assert!(
        ledger
            .find_git_operation("operation-observer")
            .unwrap()
            .is_none()
    );
    assert!(ledger.find_worktree_lease("worktree").unwrap().is_none());
}

#[test]
fn authority_claims_receipt_and_lease_then_returns_a_durable_idempotent_summary() {
    let ledger = ledger_with_run();
    let fake = FakeGit::default();
    let service = GitStatusService::new(
        ledger.clone(),
        fake.clone(),
        GitStatusAuthority::ApprovedStatus,
    );
    let request = request("success", "operation-success");
    let first = service.execute(&request).unwrap();
    let second = service.execute(&request).unwrap();
    assert_eq!(first.state, GitStatusState::Completed);
    assert_eq!(first, second);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        fake.paths.lock().unwrap().as_slice(),
        ["/canonical/worktree"]
    );
    assert_eq!(
        ledger
            .find_git_operation("operation-success")
            .unwrap()
            .unwrap()
            .phase,
        GitOperationPhase::Succeeded
    );
}

#[test]
fn a_contended_worktree_rejects_before_git_execution() {
    let ledger = ledger_with_run();
    ledger
        .claim_worktree_lease(&WorktreeLease {
            worktree_id: "worktree".into(),
            run_id: "run".into(),
            lease_token: "existing".into(),
            host_epoch: HostEpoch(1),
        })
        .unwrap();
    let fake = FakeGit::default();
    let result = GitStatusService::new(ledger, fake.clone(), GitStatusAuthority::ApprovedStatus)
        .execute(&request("contended", "operation-contended"))
        .unwrap();
    assert_eq!(result.state, GitStatusState::Rejected);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn an_accepted_restart_receipt_becomes_unprovable_without_replaying_git() {
    let ledger = ledger_with_run();
    let request = request("restart", "operation-restart");
    let command = Command {
        receipt_id: request.receipt_id.clone(),
        idempotency_key: request.idempotency_key.clone(),
        host_epoch: request.host_epoch,
        kind: "gitStatus".into(),
        payload: json!({
            "operationId": request.operation.operation_id,
            "runId": request.operation.run_id,
            "worktreeId": request.operation.worktree_id,
        }),
    };
    let accepted = Event {
        cursor: 0,
        event_id: format!("{}:git-status-accepted", command.receipt_id.0),
        receipt_id: command.receipt_id.clone(),
        host_epoch: command.host_epoch,
        kind: "gitStatusAccepted".into(),
        payload: command.payload.clone(),
    };
    assert!(matches!(
        ledger.claim_command(&command, &accepted).unwrap(),
        ReceiptClaim::Accepted(_)
    ));
    let fake = FakeGit::default();
    let result = GitStatusService::new(ledger, fake.clone(), GitStatusAuthority::ApprovedStatus)
        .execute(&request)
        .unwrap();
    assert_eq!(result.state, GitStatusState::Unprovable);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn stale_epoch_is_rejected_by_the_receipt_fence_before_git_execution() {
    let ledger = ledger_with_run();
    ledger.close_ingress(HostEpoch(1)).unwrap();
    ledger.fence_and_open(HostEpoch(1)).unwrap();
    let fake = FakeGit::default();
    let result = GitStatusService::new(ledger, fake.clone(), GitStatusAuthority::ApprovedStatus)
        .execute(&request("stale", "operation-stale"));
    assert!(result.is_err());
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn mutating_operation_is_rejected_without_reaching_the_injected_executor() {
    let ledger = ledger_with_run();
    let fake = FakeGit::default();
    let mutating = GitStatusRequest {
        operation: GitOperationRecord {
            kind: GitOperationKind::Commit,
            ..request("commit", "operation-commit").operation
        },
        ..request("commit", "operation-commit")
    };
    let result = GitStatusService::new(ledger, fake.clone(), GitStatusAuthority::ApprovedStatus)
        .execute(&mutating)
        .unwrap();
    assert_eq!(result.state, GitStatusState::Rejected);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
}
