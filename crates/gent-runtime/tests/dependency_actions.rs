use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use gent_ports::{
    DependencyActionExecutor, DependencyActionExecutorError, DependencyActionOperation, Ledger,
};
use gent_protocol::{
    DependencyAction, DependencyActionRequest, DependencyActionState, DependencyPlan,
    DependencyProvider,
};
use gent_runtime::DependencyActionService;
use gent_store::SqliteLedger;
use gent_types::{Command, Event, HostEpoch, ReceiptId, ReceiptStatus};

#[derive(Clone, Debug)]
struct FakeExecutor {
    calls: Arc<AtomicUsize>,
    fails: bool,
}

impl DependencyActionExecutor for FakeExecutor {
    fn execute(&self, _: &DependencyActionOperation) -> Result<(), DependencyActionExecutorError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fails {
            Err(DependencyActionExecutorError {
                message: "vendor failed".into(),
            })
        } else {
            Ok(())
        }
    }
}

fn plan() -> DependencyPlan {
    DependencyPlan::reviewed(
        DependencyProvider::Claude,
        DependencyAction::Install,
        "review installer",
        true,
    )
}

fn request(plan: &DependencyPlan, key: &str) -> DependencyActionRequest {
    DependencyActionRequest {
        provider: plan.provider,
        action: plan.action,
        consent_granted: true,
        receipt_id: ReceiptId(format!("receipt-{key}")),
        idempotency_key: key.into(),
        host_epoch: HostEpoch(1),
        reviewed_plan_digest: plan.reviewed_plan_digest.clone(),
    }
}

fn service(calls: Arc<AtomicUsize>) -> DependencyActionService<SqliteLedger, FakeExecutor> {
    DependencyActionService::new(
        SqliteLedger::in_memory().unwrap(),
        FakeExecutor {
            calls,
            fails: false,
        },
    )
}

#[test]
fn valid_action_settles_once_and_retries_without_another_effect() {
    let calls = Arc::new(AtomicUsize::new(0));
    let service = service(calls.clone());
    let plan = plan();
    let request = request(&plan, "once");
    let first = service.execute(&request, &plan).unwrap();
    let retry = service.execute(&request, &plan).unwrap();
    assert_eq!(first.receipt, retry.receipt);
    assert_eq!(first.receipt.status, ReceiptStatus::Settled);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn consent_denial_and_plan_mismatch_never_execute() {
    let calls = Arc::new(AtomicUsize::new(0));
    let service = service(calls.clone());
    let plan = plan();
    let mut denied = request(&plan, "denied");
    denied.consent_granted = false;
    let result = service.execute(&denied, &plan).unwrap();
    assert_eq!(result.state, DependencyActionState::ConsentRequired);
    assert_eq!(result.receipt.status, ReceiptStatus::Rejected);
    let mut mismatched = request(&plan, "mismatch");
    mismatched.reviewed_plan_digest = "different".into();
    assert_eq!(
        service.execute(&mismatched, &plan).unwrap().state,
        DependencyActionState::PlanMismatch
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn stale_or_closed_ingress_rejects_before_effect() {
    let calls = Arc::new(AtomicUsize::new(0));
    let ledger = SqliteLedger::in_memory().unwrap();
    let service = DependencyActionService::new(
        ledger.clone(),
        FakeExecutor {
            calls: calls.clone(),
            fails: false,
        },
    );
    let plan = plan();
    let mut stale = request(&plan, "stale");
    stale.host_epoch = HostEpoch(0);
    assert!(service.execute(&stale, &plan).is_err());
    ledger.close_ingress(HostEpoch(1)).unwrap();
    let closed = request(&plan, "closed");
    assert!(service.execute(&closed, &plan).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn accepted_receipt_after_restart_becomes_unprovable_without_replay() {
    let calls = Arc::new(AtomicUsize::new(0));
    let ledger = SqliteLedger::in_memory().unwrap();
    let plan = plan();
    let request = request(&plan, "crash");
    let command = Command {
        receipt_id: request.receipt_id.clone(),
        idempotency_key: request.idempotency_key.clone(),
        host_epoch: request.host_epoch,
        kind: "dependencyAction".into(),
        payload: serde_json::json!({
            "action": "install",
            "consentGranted": true,
            "provider": "claude",
            "reviewedPlanDigest": request.reviewed_plan_digest,
        }),
    };
    ledger
        .claim_command(
            &command,
            &Event {
                cursor: 0,
                event_id: "accepted".into(),
                receipt_id: command.receipt_id.clone(),
                host_epoch: command.host_epoch,
                kind: "dependencyActionAccepted".into(),
                payload: command.payload.clone(),
            },
        )
        .unwrap();
    let service = DependencyActionService::new(
        ledger,
        FakeExecutor {
            calls: calls.clone(),
            fails: false,
        },
    );
    let result = service.execute(&request, &plan).unwrap();
    assert_eq!(result.state, DependencyActionState::Unprovable);
    assert_eq!(result.receipt.status, ReceiptStatus::Unprovable);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn same_key_with_changed_request_is_rejected_by_the_ledger() {
    let calls = Arc::new(AtomicUsize::new(0));
    let service = service(calls.clone());
    let plan = plan();
    let first = request(&plan, "same");
    service.execute(&first, &plan).unwrap();
    let mut changed = request(&plan, "same");
    changed.receipt_id = ReceiptId("different-receipt".into());
    assert!(service.execute(&changed, &plan).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn known_vendor_failure_is_terminal_and_is_not_replayed() {
    let calls = Arc::new(AtomicUsize::new(0));
    let service = DependencyActionService::new(
        SqliteLedger::in_memory().unwrap(),
        FakeExecutor {
            calls: calls.clone(),
            fails: true,
        },
    );
    let plan = plan();
    let request = request(&plan, "failure");
    let first = service.execute(&request, &plan).unwrap();
    let retry = service.execute(&request, &plan).unwrap();
    assert_eq!(first.state, DependencyActionState::Failed);
    assert_eq!(first.receipt.status, ReceiptStatus::Rejected);
    assert_eq!(first.receipt, retry.receipt);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
