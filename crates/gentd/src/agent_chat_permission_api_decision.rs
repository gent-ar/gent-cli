use gent_ports::{
    AgentChatWorkspaceLedger, Ledger, PendingPermissionLedger, PolicyLedger, ReceiptClaim,
};
use gent_types::{
    PermissionDecisionRequest, PermissionDecisionResponse, PermissionDecisionResponseKind,
    PolicyScope, Receipt, ReceiptId, ReceiptStatus,
};
use sha2::{Digest, Sha256};

use super::StandaloneAgentChatPermissionPort;
use super::receipt::{permission_decision_command, permission_decision_terminal_event};

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
                let pending = self
                    .ledger
                    .pending_permission(&response.binding.conversation_id, &response.binding.run_id)
                    .map_err(|error| error.to_string())?;
                if pending
                    .as_ref()
                    .is_some_and(|request| request.binding == response.binding)
                {
                    return self.settle_receipt(&command, ReceiptStatus::Unprovable);
                }
                Err(format!("{provider} permission receipt recovery is stale"))
            }
            ReceiptClaim::Existing(receipt) => Ok(receipt),
            ReceiptClaim::Accepted(_) => {
                if let Err(error) = respond(self, response) {
                    let _ = self.settle_receipt(&command, ReceiptStatus::Unprovable);
                    return Err(error);
                }
                self.settle_receipt(&command, ReceiptStatus::Settled)
            }
        }
    }

    fn respond_codex(&self, response: &PermissionDecisionResponse) -> Result<(), String> {
        self.approved_pending(response, "Codex")?;
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
        self.approved_pending(response, "Claude")?;
        let behavior = if response.response == PermissionDecisionResponseKind::Deny {
            gent_drivers::claude_control::ClaudePermissionBehavior::Deny
        } else {
            gent_drivers::claude_control::ClaudePermissionBehavior::Allow
        };
        let persist_suggestions = matches!(
            response.response,
            PermissionDecisionResponseKind::ApproveExactTool
                | PermissionDecisionResponseKind::ApproveCategory
        );
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
        self.persist_approval(&pending, response.response)
    }

    fn persist_approval(
        &self,
        pending: &PermissionDecisionRequest,
        response: PermissionDecisionResponseKind,
    ) -> Result<(), String> {
        if matches!(
            response,
            PermissionDecisionResponseKind::Deny | PermissionDecisionResponseKind::ApproveOnce
        ) {
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
        if policy.policy_id != pending.binding.policy_id
            || policy.revision != pending.binding.policy_revision
        {
            return Err("Codex permission policy is stale".into());
        }
        let mut revised = policy.clone();
        match response {
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
            PermissionDecisionResponseKind::Deny | PermissionDecisionResponseKind::ApproveOnce => {
                return Ok(());
            }
        }
        if revised == policy {
            return Ok(());
        }
        revised.revision = policy.revision + 1;
        revised.policy_id = format!(
            "provider-permissions-v{}-{:x}",
            revised.revision,
            Sha256::digest(
                format!(
                    "{}\0{}\0{}",
                    policy.workspace_id, policy.policy_id, revised.revision
                )
                .as_bytes()
            ),
        );
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
