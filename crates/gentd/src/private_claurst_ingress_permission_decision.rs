use gent_ports::{AgentChatWorkspaceLedger, PendingPermissionLedger, PolicyLedger};
use gent_types::{
    Command, NormalizedProviderEvent, NormalizedSessionLifecycle, PermissionDecisionRequest,
    PermissionDecisionResponse, PermissionDecisionResponseKind, PolicyRecord, PolicyScope, Receipt,
    ReceiptId, ReceiptStatus,
};
use sha2::{Digest, Sha256};

use crate::agent_chat_permission_api::receipt::{
    permission_decision_command, permission_decision_terminal_event,
};
use crate::private_claurst_ingress::PrivateClaurstIngress;
use crate::private_claurst_ingress::validation::invariant;

impl<L, B> PrivateClaurstIngress<L, B>
where
    L: Clone
        + std::fmt::Debug
        + gent_ports::Ledger
        + gent_ports::GoalLedger
        + gent_ports::RunCheckpointLedger
        + gent_ports::RunLifecycleFactLedger
        + gent_ports::NormalizedSessionBatchLedger
        + PendingPermissionLedger
        + PolicyLedger
        + AgentChatWorkspaceLedger,
    B: gent_ports::PrivateClaurstBridge,
{
    pub(crate) async fn respond_permission(
        &self,
        response: PermissionDecisionResponse,
    ) -> Result<(), gent_runtime::RuntimeError> {
        let pending = self
            .ledger
            .pending_permission(&response.binding.conversation_id, &response.binding.run_id)?
            .ok_or_else(|| invariant("private Claurst permission is not pending"))?;
        if pending.binding != response.binding {
            return Err(invariant("private Claurst permission binding is stale"));
        }
        let state = self
            .sources
            .values()
            .find(|state| {
                state.binding.run_id == response.binding.run_id.0
                    && state.conversation_id.as_ref() == Some(&response.binding.conversation_id)
                    && state.turn_id.as_deref() == Some(response.binding.turn_id.as_str())
                    && !state.terminal
            })
            .cloned()
            .ok_or_else(|| invariant("private Claurst permission source is unavailable"))?;
        if !matches!(response.response, PermissionDecisionResponseKind::Deny) {
            self.persist_approval(&pending, response.response)?;
        }
        let reply = match response.response {
            PermissionDecisionResponseKind::Deny => gent_ports::ClaurstPermissionReply::Deny,
            PermissionDecisionResponseKind::ApproveOnce
            | PermissionDecisionResponseKind::ApproveExactTool
            | PermissionDecisionResponseKind::ApproveCategory => {
                gent_ports::ClaurstPermissionReply::AllowOnce
            }
        };
        self.bridge
            .respond_permission(
                state.binding.clone(),
                &response.binding.decision_id.0,
                reply,
            )
            .await?;
        self.ledger.settle_pending_permission(&response.binding)?;
        self.record_permission_lifecycle(
            &state,
            &format!("permission-{}-settled", response.binding.decision_id.0),
            NormalizedSessionLifecycle::Event {
                event: NormalizedProviderEvent::DecisionSettled {
                    decision_id: response.binding.decision_id.0,
                },
            },
            response.binding.host_epoch,
            None,
        )?;
        Ok(())
    }

    pub(crate) async fn respond_permission_with_receipt(
        &self,
        response: PermissionDecisionResponse,
        receipt_id: ReceiptId,
    ) -> Result<Receipt, gent_runtime::RuntimeError> {
        let (command, accepted) =
            permission_decision_command(&response, &receipt_id).map_err(|error| {
                gent_runtime::RuntimeError::Ledger(gent_ports::LedgerError::Storage(
                    error.to_string(),
                ))
            })?;
        match self.ledger.claim_command(&command, &accepted)? {
            gent_ports::ReceiptClaim::Existing(receipt)
                if receipt.status == ReceiptStatus::Accepted =>
            {
                let pending = self.ledger.pending_permission(
                    &response.binding.conversation_id,
                    &response.binding.run_id,
                )?;
                if pending
                    .as_ref()
                    .is_some_and(|request| request.binding == response.binding)
                {
                    return self.settle_permission_receipt(&command, ReceiptStatus::Unprovable);
                }
                Err(invariant(
                    "private Claurst permission receipt recovery is stale",
                ))
            }
            gent_ports::ReceiptClaim::Existing(receipt) => Ok(receipt),
            gent_ports::ReceiptClaim::Accepted(_) => {
                if let Err(error) = self.respond_permission(response).await {
                    let _ = self.settle_permission_receipt(&command, ReceiptStatus::Unprovable);
                    return Err(error);
                }
                self.settle_permission_receipt(&command, ReceiptStatus::Settled)
            }
        }
    }

    fn settle_permission_receipt(
        &self,
        command: &Command,
        status: ReceiptStatus,
    ) -> Result<Receipt, gent_runtime::RuntimeError> {
        let terminal = permission_decision_terminal_event(command, &status);
        self.ledger
            .settle_receipt(&command.idempotency_key, status, &terminal)
            .map_err(gent_runtime::RuntimeError::Ledger)
    }

    fn persist_approval(
        &self,
        pending: &PermissionDecisionRequest,
        response: PermissionDecisionResponseKind,
    ) -> Result<(), gent_runtime::RuntimeError> {
        let workspace = self.ledger.agent_chat_workspace_for_run(
            &pending.binding.conversation_id.0,
            &pending.binding.run_id.0,
        )?;
        let policy = self
            .ledger
            .current_policy(&workspace.workspace_id, PolicyScope::ProviderPermissions)?
            .ok_or_else(|| invariant("private Claurst permission policy is unavailable"))?;
        if policy.policy_id != pending.binding.policy_id
            || policy.revision != pending.binding.policy_revision
        {
            return Err(invariant("private Claurst permission policy is stale"));
        }
        let mut revised = policy.clone();
        match response {
            PermissionDecisionResponseKind::ApproveOnce | PermissionDecisionResponseKind::Deny => {
                return Ok(());
            }
            PermissionDecisionResponseKind::ApproveExactTool => {
                revised
                    .allowed_tools
                    .push(pending.request.tool_name.clone());
                revised.allowed_tools.sort();
                revised.allowed_tools.dedup();
            }
            PermissionDecisionResponseKind::ApproveCategory => {
                revised.allowed_categories.push(pending.request.category);
                revised.allowed_categories.sort();
                revised.allowed_categories.dedup();
            }
        }
        if revised == policy {
            return Ok(());
        }
        revised.revision = policy.revision + 1;
        revised.policy_id = revision_id(&policy, revised.revision);
        self.ledger.save_policy(&revised)?;
        Ok(())
    }
}

fn revision_id(policy: &PolicyRecord, revision: u64) -> String {
    let material = format!(
        "{}\\0{}\\0{revision}",
        policy.workspace_id, policy.policy_id
    );
    format!(
        "provider-permissions-v{revision}-{:x}",
        Sha256::digest(material.as_bytes())
    )
}
