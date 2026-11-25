use gent_ports::{
    AgentChatWorkspaceLedger, Ledger, LedgerError, PendingPermissionLedger, PolicyLedger,
    ReceiptClaim,
};
use gent_types::{
    AgentChatProvider, Command, DecisionOutcome, PermissionDecisionBinding,
    PermissionDecisionRequest, PermissionDecisionResponse, PermissionDecisionResponseKind,
    PolicyRecord, PolicyScope, Receipt, ReceiptId, ReceiptStatus,
};

use super::receipt::{
    later_answer_outcome, permission_decision_command, permission_decision_terminal_event,
    receipt_or_outcome,
};
use super::{PermissionExchangeError, StandaloneAgentChatPermissionPort};
use crate::ordinary_lifecycle_host::Undelivered;

type ProviderWrite =
    fn(&StandaloneAgentChatPermissionPort, &PermissionDecisionResponse) -> Result<(), Undelivered>;

impl StandaloneAgentChatPermissionPort {
    pub(super) fn respond_codex_with_receipt(
        &self,
        response: &PermissionDecisionResponse,
        receipt_id: &ReceiptId,
    ) -> Result<Receipt, PermissionExchangeError> {
        self.respond_with_receipt(
            response,
            receipt_id,
            AgentChatProvider::Codex,
            Self::write_codex,
        )
    }

    pub(super) fn respond_claude_with_receipt(
        &self,
        response: &PermissionDecisionResponse,
        receipt_id: &ReceiptId,
    ) -> Result<Receipt, PermissionExchangeError> {
        self.respond_with_receipt(
            response,
            receipt_id,
            AgentChatProvider::Claude,
            Self::write_claude,
        )
    }

    fn respond_with_receipt(
        &self,
        response: &PermissionDecisionResponse,
        receipt_id: &ReceiptId,
        provider: AgentChatProvider,
        write: ProviderWrite,
    ) -> Result<Receipt, PermissionExchangeError> {
        let (command, accepted) =
            permission_decision_command(response, receipt_id).map_err(|error| error.to_string())?;
        let claim = match self.ledger.claim_command(&command, &accepted) {
            Err(LedgerError::StaleEpoch {
                command: asked,
                active,
            }) if asked.0 < active.0 => {
                return Ok(self.expire(&response.binding, &command));
            }
            Err(LedgerError::Invariant(conflict)) => {
                let Ok(Some(prior)) = self.ledger.decision_receipt(&response.binding) else {
                    return Err(LedgerError::Invariant(conflict).to_string().into());
                };
                let still_pending = matches!(self.pending_for(response, provider), Ok(Some(_)));
                return match later_answer_outcome(prior, still_pending, &command) {
                    Some(receipt) => Ok(receipt),
                    None => self.answer(response, &command, provider, write),
                };
            }
            claim => claim.map_err(|error| error.to_string())?,
        };
        match claim {
            ReceiptClaim::Existing(receipt) if receipt.status == ReceiptStatus::Accepted => {
                self.resume(response, &command, provider, write)
            }
            ReceiptClaim::Existing(receipt) => Ok(receipt),
            ReceiptClaim::Accepted(_) => self.answer(response, &command, provider, write),
        }
    }

    fn answer(
        &self,
        response: &PermissionDecisionResponse,
        command: &Command,
        provider: AgentChatProvider,
        write: ProviderWrite,
    ) -> Result<Receipt, PermissionExchangeError> {
        let pending = match self.pending_for(response, provider) {
            Ok(Some(pending)) => pending,
            Ok(None) => return Ok(self.expire(&response.binding, command)),
            Err(error) => {
                let _ = self.settle_receipt(command, ReceiptStatus::Rejected);
                return Err(error.into());
            }
        };
        if unanswered_question(&pending, response) {
            let _ = self.settle_receipt(command, ReceiptStatus::Rejected);
            return Err(PermissionExchangeError::InputInvalid(
                "a question is answered with its answers as input; the request is still pending"
                    .into(),
            ));
        }
        let grant = match self.grant(&pending, response) {
            Ok(grant) => grant,
            Err(error) => {
                let _ = self.settle_receipt(command, ReceiptStatus::Rejected);
                return Err(error.into());
            }
        };
        self.deliver(
            response,
            command,
            (provider, write),
            grant.as_ref(),
            outcome(&pending, response),
        )
    }

    fn resume(
        &self,
        response: &PermissionDecisionResponse,
        command: &Command,
        provider: AgentChatProvider,
        write: ProviderWrite,
    ) -> Result<Receipt, PermissionExchangeError> {
        match self.pending_for(response, provider) {
            Ok(Some(pending)) => {
                let grant = self.grant(&pending, response)?;
                self.deliver(
                    response,
                    command,
                    (provider, write),
                    grant.as_ref(),
                    outcome(&pending, response),
                )
            }
            Ok(None) | Err(_) => Ok(self.outcome(command, ReceiptStatus::Settled)),
        }
    }

    fn deliver(
        &self,
        response: &PermissionDecisionResponse,
        command: &Command,
        (provider, write): (AgentChatProvider, ProviderWrite),
        grant: Option<&PolicyRecord>,
        outcome: DecisionOutcome,
    ) -> Result<Receipt, PermissionExchangeError> {
        match write(self, response) {
            Ok(()) => {
                if let Err(error) =
                    self.ledger
                        .settle_delivered_permission(&response.binding, grant, outcome)
                {
                    eprintln!(
                        "Permission {} was delivered; its pending row settles with the turn: {error}",
                        response.binding.decision_id.0
                    );
                } else if grant.is_some() {
                    self.ingress.permission_policy_saved();
                }
                Ok(self.outcome(command, ReceiptStatus::Settled))
            }
            Err(Undelivered::NotPending) => Ok(self.expire(&response.binding, command)),
            Err(Undelivered::InvalidInput) => {
                let _ = self.settle_receipt(command, ReceiptStatus::Rejected);
                Err(PermissionExchangeError::InputInvalid(
                    "the provider refused this answer's input; the request is still pending".into(),
                ))
            }
            Err(Undelivered::PipeBroken) => {
                let _ = self
                    .ingress
                    .interrupt_run(provider, &response.binding.run_id.0);
                Ok(self.expire(&response.binding, command))
            }
        }
    }

    fn expire(&self, binding: &PermissionDecisionBinding, command: &Command) -> Receipt {
        let _ = self.ledger.settle_pending_permission(binding);
        let settled = self
            .settle_receipt(command, ReceiptStatus::Rejected)
            .or_else(|_| self.settle_receipt(command, ReceiptStatus::Settled));
        receipt_or_outcome(command, ReceiptStatus::Rejected, settled)
    }

    fn outcome(&self, command: &Command, status: ReceiptStatus) -> Receipt {
        receipt_or_outcome(
            command,
            status.clone(),
            self.settle_receipt(command, status),
        )
    }

    fn write_codex(&self, response: &PermissionDecisionResponse) -> Result<(), Undelivered> {
        let decision = if response.response == PermissionDecisionResponseKind::Deny {
            gent_drivers::codex_control::CodexControlDecision::Deny
        } else {
            gent_drivers::codex_control::CodexControlDecision::Allow
        };
        self.ingress.respond_codex_permission(
            &response.binding.run_id.0,
            &response.binding.decision_id.0,
            decision,
            response.input.clone(),
        )
    }

    fn write_claude(&self, response: &PermissionDecisionResponse) -> Result<(), Undelivered> {
        let behavior = if response.response == PermissionDecisionResponseKind::Deny {
            gent_drivers::claude_control::ClaudePermissionBehavior::Deny
        } else {
            gent_drivers::claude_control::ClaudePermissionBehavior::Allow
        };
        let persist_suggestions = false;
        self.ingress.respond_claude_permission_with_input(
            &response.binding.run_id.0,
            &response.binding.decision_id.0,
            behavior,
            persist_suggestions,
            response.input.clone(),
        )
    }

    fn pending_for(
        &self,
        response: &PermissionDecisionResponse,
        provider: AgentChatProvider,
    ) -> Result<Option<PermissionDecisionRequest>, String> {
        match self
            .ledger
            .pending_permission(&response.binding.conversation_id, &response.binding.run_id)
            .map_err(|error| error.to_string())?
        {
            None => Ok(None),
            Some(pending) if pending.binding == response.binding => Ok(Some(pending)),
            Some(_) => Err(format!("{provider:?} permission binding is stale")),
        }
    }

    fn grant(
        &self,
        pending: &PermissionDecisionRequest,
        response: &PermissionDecisionResponse,
    ) -> Result<Option<PolicyRecord>, String> {
        if response.response == PermissionDecisionResponseKind::Deny {
            return Ok(None);
        }
        let workspace = self
            .ledger
            .agent_chat_workspace_for_run(
                &pending.binding.conversation_id.0,
                &pending.binding.run_id.0,
            )
            .map_err(|error| error.to_string())?;
        let policy = self
            .ledger
            .current_policy(&workspace.workspace_id, PolicyScope::ProviderPermissions)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "permission policy is unavailable".to_owned())?;
        super::grant::transition(&policy, pending, response).map_err(str::to_owned)
    }

    fn settle_receipt(&self, command: &Command, status: ReceiptStatus) -> Result<Receipt, String> {
        let terminal = permission_decision_terminal_event(command, &status);
        self.ledger
            .settle_receipt(&command.idempotency_key, status, &terminal)
            .map_err(|error| error.to_string())
    }
}

fn outcome(
    pending: &PermissionDecisionRequest,
    response: &PermissionDecisionResponse,
) -> DecisionOutcome {
    DecisionOutcome::answered(
        response.response,
        crate::permission_category::requires_user_response(&pending.request.tool_name),
    )
}

fn unanswered_question(
    pending: &PermissionDecisionRequest,
    response: &PermissionDecisionResponse,
) -> bool {
    response.response != PermissionDecisionResponseKind::Deny
        && response.input.is_none()
        && crate::permission_category::requires_user_response(&pending.request.tool_name)
}
