use gent_types::{
    Command, Event, PermissionDecisionBinding, PermissionDecisionResponse, Receipt, ReceiptId,
    ReceiptStatus,
};

pub(crate) fn permission_decision_command(
    response: &PermissionDecisionResponse,
    receipt_id: &ReceiptId,
) -> Result<(Command, Event), serde_json::Error> {
    let command = Command {
        receipt_id: receipt_id.clone(),
        idempotency_key: response.binding.request_idempotency_key.clone(),
        host_epoch: response.binding.host_epoch,
        kind: "agentChatPermissionDecision".into(),
        payload: serde_json::to_value(response)?,
    };
    let accepted = Event {
        cursor: 0,
        event_id: permission_decision_accepted_event_id(&response.binding),
        receipt_id: receipt_id.clone(),
        host_epoch: response.binding.host_epoch,
        kind: "agentChatPermissionDecisionAccepted".into(),
        payload: command.payload.clone(),
    };
    Ok((command, accepted))
}

pub(crate) fn permission_decision_terminal_event(
    command: &Command,
    status: &ReceiptStatus,
) -> Event {
    let status_name = match status {
        ReceiptStatus::Settled => "settled",
        ReceiptStatus::Unprovable => "unprovable",
        ReceiptStatus::Rejected => "rejected",
        ReceiptStatus::Accepted => "accepted",
    };
    Event {
        cursor: 0,
        event_id: format!("permission-decision-{status_name}:{}", command.receipt_id.0),
        receipt_id: command.receipt_id.clone(),
        host_epoch: command.host_epoch,
        kind: "agentChatPermissionDecisionTerminal".into(),
        payload: serde_json::json!({"status": status_name}),
    }
}

pub(crate) fn permission_decision_accepted_event_id(binding: &PermissionDecisionBinding) -> String {
    format!(
        "permission-decision-accepted:{}:{}:{}",
        binding.run_id.0, binding.turn_id, binding.decision_id.0
    )
}

pub(crate) fn later_answer_outcome(
    prior: Receipt,
    still_pending: bool,
    command: &Command,
) -> Option<Receipt> {
    let status = match (still_pending, &prior.status) {
        (true, ReceiptStatus::Rejected) => return None,
        (true, _) => {
            return Some(receipt_or_outcome(
                command,
                ReceiptStatus::Rejected,
                Err(()),
            ));
        }
        (false, ReceiptStatus::Accepted | ReceiptStatus::Settled) => ReceiptStatus::Settled,
        (false, _) => ReceiptStatus::Rejected,
    };
    Some(Receipt { status, ..prior })
}

pub(crate) fn receipt_or_outcome<E>(
    command: &Command,
    status: ReceiptStatus,
    settlement: Result<Receipt, E>,
) -> Receipt {
    settlement.unwrap_or_else(|_| Receipt {
        receipt_id: command.receipt_id.clone(),
        idempotency_key: command.idempotency_key.clone(),
        status,
        host_epoch: command.host_epoch,
    })
}
