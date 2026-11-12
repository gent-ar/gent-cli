use gent_drivers::{
    codex_control::{CodexControlDecision, CodexControlRequest},
    public_protocol::PublicWireFact,
};
use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, NormalizedSessionBatchLedger, PendingPermissionLedger,
    PolicyLedger, PublicProviderResolver, TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::{
    AgentChatConversationId, AgentChatDecisionId, AgentChatRunId, NormalizedLifecycleSignal,
    PermissionDecisionBinding, PermissionDecisionRequest, PermissionRequest,
    PermissionRequestDigest, ToolActivity, ToolPhase, TurnPhase,
};
use sha2::{Digest, Sha256};

use super::{CodexPromptExecution, CodexPromptLifecycle, missing_binding};

impl<L, D, R> CodexPromptLifecycle<L, D, R>
where
    L: Clone
        + Ledger
        + gent_ports::RunLifecycleFactLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + NormalizedSessionBatchLedger
        + AgentChatPromptDispatchLedger
        + gent_ports::AttachmentLedger
        + gent_ports::AgentChatReadLedger
        + AgentChatRunContextReader
        + ConversationContentReader
        + gent_ports::AgentChatWorkspaceLedger
        + PendingPermissionLedger
        + PolicyLedger
        + gent_ports::AttachmentLedger
        + gent_ports::ToolSourceLedger
        + gent_ports::AgentChatConversationConfigLedger,
    D: CodexPromptExecution + Clone,
    R: PublicProviderResolver,
{
    pub(super) fn record_permission_request(
        &mut self,
        run_id: &str,
        host_epoch: gent_types::HostEpoch,
        request: CodexControlRequest,
    ) -> Result<u16, RuntimeError> {
        let binding = self
            .active
            .get(run_id)
            .cloned()
            .ok_or_else(missing_binding)?;
        let ledger = self.runtime.ledger();
        // The polling epoch comes from the composed owner, not provider/client input.
        // Fence before sending any auto-approval if a successor closed/replaced this owner.
        let ingress = ledger.host_ingress()?;
        if ingress.epoch != host_epoch || ingress.mode != gent_ports::IngressMode::Open {
            return Err(missing_binding());
        }
        let policy = binding.permission.clone();
        let category = crate::permission_category::for_tool(&request.tool_name);
        let normalized = PermissionRequest::new(
            request.tool_name.clone(),
            category,
            request.input.clone(),
            None,
        );
        let config = ledger.current_conversation_config(&binding.prompt.message.conversation_id)?;
        let disallowed = config
            .as_ref()
            .map_or(&[][..], |config| config.disallowed_tools.as_slice());
        let mode = gent_runtime::AgentChatReadService::new(ledger.clone())
            .run_selection(&binding.prompt.message.conversation_id, run_id)?
            .mode;
        let automatic =
            if crate::permission_preflight::read_only_denial(mode, &normalized).is_some() {
                Some(CodexControlDecision::Deny)
            } else {
                match self.runner.permission_preflight(
                    &crate::permission_preflight::PreflightContext {
                        run_id,
                        conversation_id: &binding.prompt.message.conversation_id,
                        disallowed,
                        injection: self.runtime.internal_mcp(),
                        host_epoch,
                    },
                    &policy,
                    &normalized,
                ) {
                    crate::permission_preflight::PermissionPreflight::Allow => {
                        Some(CodexControlDecision::Allow)
                    }
                    crate::permission_preflight::PermissionPreflight::Deny => {
                        Some(CodexControlDecision::Deny)
                    }
                    crate::permission_preflight::PermissionPreflight::Ask => None,
                }
            };
        if let Some(decision) = automatic {
            self.runner
                .respond_codex_control(run_id, &request.request_key, decision, None)
                .map_err(RuntimeError::from)?;
            self.record_wire(
                run_id,
                host_epoch,
                &PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity {
                    activity: ToolActivity {
                        tool_use_id: request.tool_use_id,
                        tool_name: request.tool_name,
                        phase: if decision == CodexControlDecision::Allow {
                            ToolPhase::Started
                        } else {
                            ToolPhase::Failed
                        },
                        output_digest: None,
                        input: request
                            .input
                            .as_ref()
                            .and_then(gent_types::operation_preview),
                        parent_tool_use_id: None,
                    },
                }),
            )?;
            return Ok(1);
        }
        let bytes = serde_json::to_vec(&normalized).map_err(|error| {
            RuntimeError::Ledger(gent_ports::LedgerError::Storage(error.to_string()))
        })?;
        ledger.save_pending_permission(&PermissionDecisionRequest {
            binding: PermissionDecisionBinding {
                decision_id: AgentChatDecisionId(request.request_key.clone()),
                request_idempotency_key: permission_idempotency_key(
                    run_id,
                    &binding.prompt.message.turn_id,
                    &request.request_key,
                ),
                conversation_id: AgentChatConversationId(
                    binding.prompt.message.conversation_id.clone(),
                ),
                run_id: AgentChatRunId(run_id.into()),
                turn_id: binding.prompt.message.turn_id.clone(),
                policy_id: policy.policy_id,
                policy_revision: policy.revision,
                host_epoch,
                request_digest_sha256: PermissionRequestDigest(format!(
                    "{:x}",
                    Sha256::digest(bytes)
                )),
            },
            request: normalized,
        })?;
        for fact in [
            PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootPhase {
                phase: TurnPhase::WaitingPermission,
            }),
            PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity {
                activity: ToolActivity {
                    tool_use_id: request.tool_use_id,
                    tool_name: request.tool_name,
                    phase: ToolPhase::WaitingPermission,
                    output_digest: None,
                    input: request
                        .input
                        .as_ref()
                        .and_then(gent_types::operation_preview),
                    parent_tool_use_id: None,
                },
            }),
        ] {
            self.record_wire(run_id, host_epoch, &fact)?;
        }
        Ok(2)
    }

    pub(crate) fn respond_permission(
        &self,
        run_id: &str,
        request_id: &str,
        decision: gent_drivers::codex_control::CodexControlDecision,
        answers: Option<serde_json::Value>,
    ) -> Result<(), RuntimeError> {
        if !self.active.contains_key(run_id) {
            return Err(missing_binding());
        }
        self.runner
            .respond_codex_control(run_id, request_id, decision, answers)
            .map_err(RuntimeError::from)
    }
}

fn permission_idempotency_key(run_id: &str, turn_id: &str, request_key: &str) -> String {
    format!("codex:{run_id}:{turn_id}:{request_key}")
}

#[cfg(test)]
mod tests {
    use super::permission_idempotency_key;

    #[test]
    fn request_keys_are_scoped_to_their_run_and_turn() {
        assert_ne!(
            permission_idempotency_key("run-a", "turn-1", "0"),
            permission_idempotency_key("run-b", "turn-1", "0")
        );
        assert_ne!(
            permission_idempotency_key("run-a", "turn-1", "0"),
            permission_idempotency_key("run-a", "turn-2", "0")
        );
    }
}
