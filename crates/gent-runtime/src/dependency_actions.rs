//! Receipt-backed orchestration for explicit public dependency effects.

use std::sync::{Arc, Mutex};

use gent_ports::{DependencyActionExecutor, DependencyActionOperation, Ledger, ReceiptClaim};
use gent_protocol::{
    DependencyActionRequest, DependencyActionResult, DependencyActionState, DependencyPlan,
    dependency_plan_digest,
};
use gent_types::{Command, Event, Receipt, ReceiptStatus};

use crate::RuntimeError;

/// Serializes external dependency effects while durable receipts make retries safe across restarts.
#[derive(Clone, Debug)]
pub struct DependencyActionService<L, E> {
    ledger: L,
    executor: E,
    serial: Arc<Mutex<()>>,
}

impl<L, E> DependencyActionService<L, E> {
    /// Creates a receipt-backed dependency action service.
    #[must_use]
    pub fn new(ledger: L, executor: E) -> Self {
        Self {
            ledger,
            executor,
            serial: Arc::new(Mutex::new(())),
        }
    }
}

impl<L: Ledger, E: DependencyActionExecutor> DependencyActionService<L, E> {
    /// Claims, validates, and terminally records one explicit dependency operation.
    ///
    /// An accepted receipt found after a process restart is never replayed: it becomes
    /// `Unprovable`, because the prior external process may already have run.
    ///
    /// # Errors
    /// Returns an error when the host fence rejects the request or durable receipt persistence fails.
    pub fn execute(
        &self,
        request: &DependencyActionRequest,
        plan: &DependencyPlan,
    ) -> Result<DependencyActionResult, RuntimeError> {
        let _serial = self
            .serial
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let command = dependency_action_command(request);
        let accepted = accepted_event(&command);
        match self.ledger.claim_command(&command, &accepted)? {
            ReceiptClaim::Accepted(receipt) => self.execute_claimed(request, plan, &receipt),
            ReceiptClaim::Existing(receipt) if receipt.status == ReceiptStatus::Accepted => self
                .settle(
                    plan,
                    &receipt,
                    ReceiptStatus::Unprovable,
                    DependencyActionState::Unprovable,
                    None,
                ),
            ReceiptClaim::Existing(receipt) => Ok(DependencyActionResult {
                plan: plan.clone(),
                state: existing_state(&receipt),
                receipt,
                detail: None,
            }),
        }
    }

    fn execute_claimed(
        &self,
        request: &DependencyActionRequest,
        plan: &DependencyPlan,
        receipt: &Receipt,
    ) -> Result<DependencyActionResult, RuntimeError> {
        if !request.consent_granted {
            return self.settle(
                plan,
                receipt,
                ReceiptStatus::Rejected,
                DependencyActionState::ConsentRequired,
                None,
            );
        }
        if !matches_request(request, plan) {
            return self.settle(
                plan,
                receipt,
                ReceiptStatus::Rejected,
                DependencyActionState::PlanMismatch,
                None,
            );
        }
        let operation = DependencyActionOperation {
            provider: request.provider.as_str().into(),
            action: action_name(request.action).into(),
        };
        match self.executor.execute(&operation) {
            Ok(()) => self.settle(
                plan,
                receipt,
                ReceiptStatus::Settled,
                DependencyActionState::Completed,
                None,
            ),
            Err(error) => self.settle(
                plan,
                receipt,
                ReceiptStatus::Rejected,
                DependencyActionState::Failed,
                Some(error.to_string()),
            ),
        }
    }

    fn settle(
        &self,
        plan: &DependencyPlan,
        receipt: &Receipt,
        status: ReceiptStatus,
        state: DependencyActionState,
        detail: Option<String>,
    ) -> Result<DependencyActionResult, RuntimeError> {
        let terminal = Event {
            cursor: 0,
            event_id: format!("{}:dependency-terminal", receipt.receipt_id.0),
            receipt_id: receipt.receipt_id.clone(),
            host_epoch: receipt.host_epoch,
            kind: terminal_kind(state).into(),
            payload: serde_json::json!({ "status": status }),
        };
        let receipt = self
            .ledger
            .settle_receipt(&receipt.idempotency_key, status, &terminal)?;
        Ok(DependencyActionResult {
            plan: plan.clone(),
            state,
            receipt,
            detail,
        })
    }
}

/// Returns the one durable command identity for a reviewed dependency action.
#[must_use]
pub fn dependency_action_command(request: &DependencyActionRequest) -> Command {
    Command {
        receipt_id: request.receipt_id.clone(),
        idempotency_key: request.idempotency_key.clone(),
        host_epoch: request.host_epoch,
        kind: "dependencyAction".into(),
        payload: serde_json::json!({
            "action": action_name(request.action),
            "consentGranted": request.consent_granted,
            "provider": request.provider.as_str(),
            "reviewedPlanDigest": request.reviewed_plan_digest,
        }),
    }
}

fn accepted_event(command: &Command) -> Event {
    Event {
        cursor: 0,
        event_id: format!("{}:dependency-accepted", command.receipt_id.0),
        receipt_id: command.receipt_id.clone(),
        host_epoch: command.host_epoch,
        kind: "dependencyActionAccepted".into(),
        payload: command.payload.clone(),
    }
}

fn matches_request(request: &DependencyActionRequest, plan: &DependencyPlan) -> bool {
    plan.provider == request.provider
        && plan.action == request.action
        && plan.consent_required
        && plan.reviewed_plan_digest == request.reviewed_plan_digest
        && plan.reviewed_plan_digest
            == dependency_plan_digest(
                plan.provider,
                plan.action,
                &plan.instruction,
                plan.consent_required,
            )
}

fn action_name(action: gent_protocol::DependencyAction) -> &'static str {
    match action {
        gent_protocol::DependencyAction::Install => "install",
        gent_protocol::DependencyAction::Update => "update",
    }
}

fn existing_state(receipt: &Receipt) -> DependencyActionState {
    match receipt.status {
        ReceiptStatus::Settled => DependencyActionState::Completed,
        ReceiptStatus::Unprovable => DependencyActionState::Unprovable,
        ReceiptStatus::Accepted | ReceiptStatus::Rejected => DependencyActionState::Failed,
    }
}

fn terminal_kind(state: DependencyActionState) -> &'static str {
    match state {
        DependencyActionState::ConsentRequired => "dependencyActionConsentRequired",
        DependencyActionState::Completed => "dependencyActionCompleted",
        DependencyActionState::Failed => "dependencyActionFailed",
        DependencyActionState::PlanMismatch => "dependencyActionPlanMismatch",
        DependencyActionState::Unprovable => "dependencyActionUnprovable",
    }
}
