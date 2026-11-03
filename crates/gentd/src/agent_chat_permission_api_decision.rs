use gent_ports::{
    AgentChatWorkspaceLedger, Ledger, PendingPermissionLedger, PolicyLedger, ReceiptClaim,
};
use gent_types::{
    PermissionDecisionRequest, PermissionDecisionResponse, PermissionDecisionResponseKind,
    PolicyScope, Receipt, ReceiptId, ReceiptStatus,
};

use super::StandaloneAgentChatPermissionPort;
use super::receipt::{
    permission_decision_command, permission_decision_terminal_event, receipt_or_unknown,
};

type ProviderResponder =
    fn(&StandaloneAgentChatPermissionPort, &PermissionDecisionResponse) -> Result<(), String>;

impl StandaloneAgentChatPermissionPort {
    pub(super) fn respond_codex_with_receipt(
        &self,
        response: &PermissionDecisionResponse,
        receipt_id: &ReceiptId,
    ) -> Result<Receipt, String> {
        self.respond_with_receipt(response, receipt_id, "Codex", Self::respond_codex)
    }

    pub(super) fn respond_claude_with_receipt(
        &self,
        response: &PermissionDecisionResponse,
        receipt_id: &ReceiptId,
    ) -> Result<Receipt, String> {
        self.respond_with_receipt(response, receipt_id, "Claude", Self::respond_claude)
    }

    fn respond_with_receipt(
        &self,
        response: &PermissionDecisionResponse,
        receipt_id: &ReceiptId,
        provider: &str,
        respond: ProviderResponder,
    ) -> Result<Receipt, String> {
        let (command, accepted) =
            permission_decision_command(response, receipt_id).map_err(|error| error.to_string())?;
        match self
            .ledger
            .claim_command(&command, &accepted)
            .map_err(|error| error.to_string())?
        {
            ReceiptClaim::Existing(receipt) if receipt.status == ReceiptStatus::Accepted => {
                // A crash can occur after the provider consumed the decision and
                // removed the pending request but before the receipt settled.
                Ok(receipt_or_unknown(
                    &command,
                    self.settle_receipt(&command, ReceiptStatus::Unprovable),
                ))
            }
            ReceiptClaim::Existing(receipt) => Ok(receipt),
            ReceiptClaim::Accepted(_) => {
                // Validate the binding and persist any scoped grant before the
                // provider write, so a stale decision is a definite rejection.
                if let Err(error) = self.approved_pending(response, provider) {
                    let _ = self.settle_receipt(&command, ReceiptStatus::Rejected);
                    return Err(error);
                }
                if respond(self, response).is_err() {
                    // The provider write may already have happened. Return the durable
                    // uncertainty rather than an error that invites a new decision.
                    return Ok(receipt_or_unknown(
                        &command,
                        self.settle_receipt(&command, ReceiptStatus::Unprovable),
                    ));
                }
                Ok(receipt_or_unknown(
                    &command,
                    self.settle_receipt(&command, ReceiptStatus::Settled),
                ))
            }
        }
    }

    fn respond_codex(&self, response: &PermissionDecisionResponse) -> Result<(), String> {
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
        )?;
        self.ledger
            .settle_pending_permission(&response.binding)
            .map_err(|error| error.to_string())
    }

    fn respond_claude(&self, response: &PermissionDecisionResponse) -> Result<(), String> {
        let behavior = if response.response == PermissionDecisionResponseKind::Deny {
            gent_drivers::claude_control::ClaudePermissionBehavior::Deny
        } else {
            gent_drivers::claude_control::ClaudePermissionBehavior::Allow
        };
        // Gent has already persisted precisely the scope selected by the user.
        // Provider suggestions are not that grant: they may describe a broader
        // rule or a different storage destination, and could outlive changes to
        // Gent's policy. Keep subsequent decisions on the shared policy path.
        let persist_suggestions = false;
        self.ingress.respond_claude_permission_with_input(
            &response.binding.run_id.0,
            &response.binding.decision_id.0,
            behavior,
            persist_suggestions,
            response.input.clone(),
        )?;
        self.ledger
            .settle_pending_permission(&response.binding)
            .map_err(|error| error.to_string())
    }

    fn approved_pending(
        &self,
        response: &PermissionDecisionResponse,
        provider: &str,
    ) -> Result<(), String> {
        let pending = self
            .ledger
            .pending_permission(&response.binding.conversation_id, &response.binding.run_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("{provider} permission is not pending"))?;
        if pending.binding != response.binding {
            return Err(format!("{provider} permission binding is stale"));
        }
        self.persist_approval(&pending, response)
    }

    fn persist_approval(
        &self,
        pending: &PermissionDecisionRequest,
        response: &PermissionDecisionResponse,
    ) -> Result<(), String> {
        if response.response == PermissionDecisionResponseKind::Deny {
            return Ok(());
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
            .ok_or_else(|| "Codex permission policy is unavailable".to_owned())?;
        let Some(revised) =
            super::grant::transition(&policy, pending, response).map_err(str::to_owned)?
        else {
            return Ok(());
        };
        self.ledger
            .save_policy(&revised)
            .map_err(|error| error.to_string())
    }

    fn settle_receipt(
        &self,
        command: &gent_types::Command,
        status: ReceiptStatus,
    ) -> Result<Receipt, String> {
        let terminal = permission_decision_terminal_event(command, &status);
        self.ledger
            .settle_receipt(&command.idempotency_key, status, &terminal)
            .map_err(|error| error.to_string())
    }
}
