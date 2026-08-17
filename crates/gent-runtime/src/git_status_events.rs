//! Durable command and event representations for the fixed Git status effect.

use gent_ports::GitStatusSummary;
use gent_types::{Command, Event, ReceiptId, ReceiptStatus};

use crate::{GitStatusRequest, GitStatusState};

pub(super) fn command_for(request: &GitStatusRequest) -> Command {
    Command {
        receipt_id: request.receipt_id.clone(),
        idempotency_key: request.idempotency_key.clone(),
        host_epoch: request.host_epoch,
        kind: "gitStatus".into(),
        payload: serde_json::json!({
            "operationId": request.operation.operation_id,
            "runId": request.operation.run_id,
            "worktreeId": request.operation.worktree_id,
        }),
    }
}

pub(super) fn accepted_event(command: &Command) -> Event {
    Event {
        cursor: 0,
        event_id: format!("{}:git-status-accepted", command.receipt_id.0),
        receipt_id: command.receipt_id.clone(),
        host_epoch: command.host_epoch,
        kind: "gitStatusAccepted".into(),
        payload: command.payload.clone(),
    }
}

pub(super) fn terminal_event_id(receipt_id: &ReceiptId) -> String {
    format!("{}:git-status-terminal", receipt_id.0)
}

pub(super) const fn receipt_status(state: GitStatusState) -> ReceiptStatus {
    match state {
        GitStatusState::Completed => ReceiptStatus::Settled,
        GitStatusState::Unprovable => ReceiptStatus::Unprovable,
        GitStatusState::DeniedObserver | GitStatusState::Failed | GitStatusState::Rejected => {
            ReceiptStatus::Rejected
        }
    }
}

pub(super) const fn terminal_kind(state: GitStatusState) -> &'static str {
    match state {
        GitStatusState::Completed => "gitStatusCompleted",
        GitStatusState::Unprovable => "gitStatusUnprovable",
        GitStatusState::DeniedObserver => "gitStatusDeniedObserver",
        GitStatusState::Failed => "gitStatusFailed",
        GitStatusState::Rejected => "gitStatusRejected",
    }
}

pub(super) fn summary_payload(summary: Option<&GitStatusSummary>) -> serde_json::Value {
    summary.map_or_else(
        || serde_json::json!({}),
        |summary| {
            serde_json::json!({
                "entryCount": summary.entry_count,
                "outputDigestSha256": summary.output_digest_sha256,
            })
        },
    )
}
