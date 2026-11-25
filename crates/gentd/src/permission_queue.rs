use std::collections::VecDeque;

use gent_ports::{LedgerError, PendingPermissionLedger};
use gent_types::{
    AgentChatConversationId, AgentChatRunId, PermissionDecisionRequest, PolicyRecord,
};

pub(crate) fn pending<L: PendingPermissionLedger>(
    ledger: &L,
    conversation_id: &str,
    run_id: &str,
) -> Result<Option<PermissionDecisionRequest>, LedgerError> {
    ledger.pending_permission(
        &AgentChatConversationId(conversation_id.into()),
        &AgentChatRunId(run_id.into()),
    )
}

pub(crate) fn waits_behind<L: PendingPermissionLedger, T>(
    ledger: &L,
    conversation_id: &str,
    run_id: &str,
    decision_id: &str,
    queue: &VecDeque<T>,
) -> Result<bool, LedgerError> {
    if !queue.is_empty() {
        return Ok(true);
    }
    Ok(pending(ledger, conversation_id, run_id)?
        .is_some_and(|request| request.binding.decision_id.0 != decision_id))
}

pub(crate) fn next_ready<L: PendingPermissionLedger, T>(
    ledger: &L,
    conversation_id: &str,
    run_id: &str,
    queue: &mut VecDeque<T>,
) -> Result<Option<T>, LedgerError> {
    if queue.is_empty() || pending(ledger, conversation_id, run_id)?.is_some() {
        return Ok(None);
    }
    Ok(queue.pop_front())
}

pub(crate) fn reconsiders(
    pending: &PermissionDecisionRequest,
    policy: &PolicyRecord,
    reconsidered: &mut Option<u64>,
) -> bool {
    let changed = pending.binding.policy_revision != policy.revision
        && *reconsidered != Some(policy.revision);
    if changed {
        *reconsidered = Some(policy.revision);
    }
    changed
}
