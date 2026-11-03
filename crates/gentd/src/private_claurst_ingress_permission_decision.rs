use gent_ports::{AgentChatWorkspaceLedger, PendingPermissionLedger, PolicyLedger};
use gent_types::{
    Command, NormalizedProviderEvent, NormalizedSessionLifecycle, PermissionDecisionRequest,
    PermissionDecisionResponse, PermissionDecisionResponseKind, PolicyScope, Receipt, ReceiptId,
    ReceiptStatus,
};

use crate::agent_chat_permission_api::receipt::{
    permission_decision_command, permission_decision_terminal_event, receipt_or_unknown,
};
use crate::private_claurst_ingress::validation::invariant;
use crate::private_claurst_ingress::{BoundSource, PrivateClaurstIngress};

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
        let state = self.prepare_permission(&response)?;
        self.dispatch_permission(&response, state).await
    }

    fn prepare_permission(
        &self,
        response: &PermissionDecisionResponse,
    ) -> Result<BoundSource, gent_runtime::RuntimeError> {
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
            self.persist_approval(&pending, response)?;
        }
        Ok(state)
    }

    async fn dispatch_permission(
        &self,
        response: &PermissionDecisionResponse,
        state: BoundSource,
    ) -> Result<(), gent_runtime::RuntimeError> {
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
                    decision_id: response.binding.decision_id.0.clone(),
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
                Ok(receipt_or_unknown(
                    &command,
                    self.settle_permission_receipt(&command, ReceiptStatus::Unprovable),
                ))
            }
            gent_ports::ReceiptClaim::Existing(receipt) => Ok(receipt),
            gent_ports::ReceiptClaim::Accepted(_) => {
                let state = match self.prepare_permission(&response) {
                    Ok(state) => state,
                    Err(error) => {
                        let _ = self.settle_permission_receipt(&command, ReceiptStatus::Rejected);
                        return Err(error);
                    }
                };
                if self.dispatch_permission(&response, state).await.is_err() {
                    return Ok(receipt_or_unknown(
                        &command,
                        self.settle_permission_receipt(&command, ReceiptStatus::Unprovable),
                    ));
                }
                Ok(receipt_or_unknown(
                    &command,
                    self.settle_permission_receipt(&command, ReceiptStatus::Settled),
                ))
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
        response: &PermissionDecisionResponse,
    ) -> Result<(), gent_runtime::RuntimeError> {
        let workspace = self.ledger.agent_chat_workspace_for_run(
            &pending.binding.conversation_id.0,
            &pending.binding.run_id.0,
        )?;
        let policy = self
            .ledger
            .current_policy(&workspace.workspace_id, PolicyScope::ProviderPermissions)?
            .ok_or_else(|| invariant("private Claurst permission policy is unavailable"))?;
        let Some(revised) =
            crate::agent_chat_permission_api::grant::transition(&policy, pending, response)
                .map_err(invariant)?
        else {
            return Ok(());
        };
        self.ledger.save_policy(&revised)?;
        Ok(())
    }
}
