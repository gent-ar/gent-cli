use gent_ports::{AgentChatWorkspaceLedger, PendingPermissionLedger, PolicyLedger};
use gent_types::{
    Command, PermissionDecisionRequest, PermissionDecisionResponse, PermissionDecisionResponseKind,
    PolicyRecord, PolicyScope, Receipt, ReceiptId, ReceiptStatus,
};

use crate::agent_chat_permission_api::receipt::{
    later_answer_outcome, permission_decision_command, permission_decision_terminal_event,
    receipt_or_outcome,
};
use crate::private_claurst_ingress::validation::invariant;
use crate::private_claurst_ingress::{BoundSource, PrivateClaurstIngress};

type PreparedPermission = (
    BoundSource,
    Option<PolicyRecord>,
    gent_types::DecisionOutcome,
);

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
        + AgentChatWorkspaceLedger
        + gent_ports::TranscriptLedger
        + gent_ports::AgentChatConversationConfigLedger
        + gent_ports::AgentChatReadLedger,
    B: gent_ports::PrivateClaurstBridge,
{
    pub(crate) async fn respond_permission(
        &self,
        response: PermissionDecisionResponse,
    ) -> Result<(), gent_runtime::RuntimeError> {
        let (state, grant, outcome) = self
            .prepare_permission(&response)?
            .ok_or_else(|| invariant("private Claurst permission is not pending"))?;
        self.write_permission(&response, &state).await?;
        self.settle_written(&response, grant.as_ref(), outcome)
    }

    fn prepare_permission(
        &self,
        response: &PermissionDecisionResponse,
    ) -> Result<Option<PreparedPermission>, gent_runtime::RuntimeError> {
        let Some(pending) = self
            .ledger
            .pending_permission(&response.binding.conversation_id, &response.binding.run_id)?
        else {
            return Ok(None);
        };
        if pending.binding != response.binding {
            return Err(invariant("private Claurst permission binding is stale"));
        }
        let Some(state) = self
            .sources
            .values()
            .find(|state| {
                state.binding.run_id == response.binding.run_id.0
                    && state.conversation_id.as_ref() == Some(&response.binding.conversation_id)
                    && state.turn_id.as_deref() == Some(response.binding.turn_id.as_str())
                    && !state.terminal
            })
            .cloned()
        else {
            return Ok(None);
        };
        let grant = if matches!(response.response, PermissionDecisionResponseKind::Deny) {
            None
        } else {
            self.grant(&pending, response)?
        };
        let outcome = gent_types::DecisionOutcome::answered(
            response.response,
            crate::permission_category::requires_user_response(&pending.request.tool_name),
        );
        Ok(Some((state, grant, outcome)))
    }

    async fn write_permission(
        &self,
        response: &PermissionDecisionResponse,
        state: &BoundSource,
    ) -> Result<(), gent_runtime::RuntimeError> {
        let reply = match response.response {
            PermissionDecisionResponseKind::Deny => gent_ports::ClaurstPermissionReply::Deny,
            PermissionDecisionResponseKind::ApproveOnce
            | PermissionDecisionResponseKind::ApproveExactTool
            | PermissionDecisionResponseKind::ApproveCategory
            | PermissionDecisionResponseKind::ApproveCommandPattern => {
                gent_ports::ClaurstPermissionReply::AllowOnce
            }
        };
        self.bridge
            .respond_permission(
                state.binding.clone(),
                &response.binding.decision_id.0,
                reply,
            )
            .await
            .map_err(Into::into)
    }

    fn settle_written(
        &self,
        response: &PermissionDecisionResponse,
        grant: Option<&PolicyRecord>,
        outcome: gent_types::DecisionOutcome,
    ) -> Result<(), gent_runtime::RuntimeError> {
        self.ledger
            .settle_delivered_permission(&response.binding, grant, outcome)?;
        Ok(())
    }

    pub(crate) async fn respond_permission_with_receipt(
        &mut self,
        response: PermissionDecisionResponse,
        receipt_id: ReceiptId,
    ) -> Result<Receipt, gent_runtime::RuntimeError> {
        let (command, accepted) =
            permission_decision_command(&response, &receipt_id).map_err(|error| {
                gent_runtime::RuntimeError::Ledger(gent_ports::LedgerError::Storage(
                    error.to_string(),
                ))
            })?;
        let fresh = match self.ledger.claim_command(&command, &accepted) {
            Err(gent_ports::LedgerError::StaleEpoch {
                command: asked,
                active,
            }) if asked.0 < active.0 => {
                return Ok(self.expire_permission(&response, &command));
            }
            Err(gent_ports::LedgerError::Invariant(conflict)) => {
                let Ok(Some(prior)) = self.ledger.decision_receipt(&response.binding) else {
                    return Err(gent_ports::LedgerError::Invariant(conflict).into());
                };
                if let Some(receipt) =
                    later_answer_outcome(prior, self.still_pending(&response), &command)
                {
                    return Ok(receipt);
                }
                true
            }
            Err(error) => return Err(error.into()),
            Ok(gent_ports::ReceiptClaim::Existing(receipt))
                if receipt.status != ReceiptStatus::Accepted =>
            {
                return Ok(receipt);
            }
            Ok(gent_ports::ReceiptClaim::Existing(_)) => false,
            Ok(gent_ports::ReceiptClaim::Accepted(_)) => true,
        };
        let (state, grant, outcome) = match self.prepare_permission(&response) {
            Ok(Some(prepared)) => prepared,
            Ok(None) if fresh || self.still_pending(&response) => {
                return Ok(self.expire_permission(&response, &command));
            }
            Ok(None) => return Ok(self.permission_outcome(&command, ReceiptStatus::Settled)),
            Err(error) if fresh => {
                let _ = self.settle_permission_receipt(&command, ReceiptStatus::Rejected);
                return Err(error);
            }
            Err(_) => return Ok(self.permission_outcome(&command, ReceiptStatus::Settled)),
        };
        match self.write_permission(&response, &state).await {
            Ok(()) => {}
            Err(gent_runtime::RuntimeError::Port(gent_ports::PortError::NotPending(_))) => {
                return Ok(self.expire_permission(&response, &command));
            }
            Err(
                error @ gent_runtime::RuntimeError::Port(gent_ports::PortError::InvalidInput(_)),
            ) => {
                let _ = self.settle_permission_receipt(&command, ReceiptStatus::Rejected);
                return Err(error);
            }
            Err(_) => {
                let _ = self.cancel_run(&response.binding.run_id.0).await;
                return Ok(self.expire_permission(&response, &command));
            }
        }
        if let Err(error) = self.settle_written(&response, grant.as_ref(), outcome) {
            eprintln!(
                "Claurst permission {} was delivered; its pending row settles with the turn: {error}",
                response.binding.decision_id.0
            );
        }
        Ok(self.permission_outcome(&command, ReceiptStatus::Settled))
    }

    fn still_pending(&self, response: &PermissionDecisionResponse) -> bool {
        self.ledger
            .pending_permission(&response.binding.conversation_id, &response.binding.run_id)
            .map_or(true, |pending| {
                pending.is_some_and(|pending| pending.binding == response.binding)
            })
    }

    fn expire_permission(
        &self,
        response: &PermissionDecisionResponse,
        command: &Command,
    ) -> Receipt {
        let _ = self.ledger.settle_pending_permission(&response.binding);
        let settled = self
            .settle_permission_receipt(command, ReceiptStatus::Rejected)
            .or_else(|_| self.settle_permission_receipt(command, ReceiptStatus::Settled));
        receipt_or_outcome(command, ReceiptStatus::Rejected, settled)
    }

    fn permission_outcome(&self, command: &Command, status: ReceiptStatus) -> Receipt {
        receipt_or_outcome(
            command,
            status.clone(),
            self.settle_permission_receipt(command, status),
        )
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

    fn grant(
        &self,
        pending: &PermissionDecisionRequest,
        response: &PermissionDecisionResponse,
    ) -> Result<Option<PolicyRecord>, gent_runtime::RuntimeError> {
        let workspace = self.ledger.agent_chat_workspace_for_run(
            &pending.binding.conversation_id.0,
            &pending.binding.run_id.0,
        )?;
        let policy = self
            .ledger
            .current_policy(&workspace.workspace_id, PolicyScope::ProviderPermissions)?
            .ok_or_else(|| invariant("private Claurst permission policy is unavailable"))?;
        crate::agent_chat_permission_api::grant::transition(&policy, pending, response)
            .map_err(invariant)
    }
}
