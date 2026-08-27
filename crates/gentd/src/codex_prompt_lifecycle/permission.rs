use gent_drivers::{codex_control::CodexControlRequest, public_protocol::PublicWireFact};
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
        let workspace =
            ledger.agent_chat_workspace_for_run(&binding.prompt.message.conversation_id, run_id)?;
        let policy = crate::permission_workspace::policy_for(&ledger, &workspace.workspace_id)?;
        let category = crate::permission_category::for_tool(&request.tool_name);
        let normalized = PermissionRequest {
            tool_name: request.tool_name.clone(),
            category,
            input: request.input.clone(),
        };
        match crate::permission_preflight::evaluate(&policy, &normalized) {
            crate::permission_preflight::PermissionPreflight::Allow => {
                self.runner
                    .respond_codex_control(
                        run_id,
                        &request.request_key,
                        gent_drivers::codex_control::CodexControlDecision::Allow,
                        None,
                    )
                    .map_err(RuntimeError::from)?;
                self.record_wire(
                    run_id,
                    host_epoch,
                    &PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity {
                        activity: ToolActivity {
                            tool_use_id: request.tool_use_id,
                            tool_name: request.tool_name,
                            phase: ToolPhase::Started,
                            output_digest: None,
                        },
                    }),
                )?;
                return Ok(1);
            }
            crate::permission_preflight::PermissionPreflight::Deny => {
                self.runner
                    .respond_codex_control(
                        run_id,
                        &request.request_key,
                        gent_drivers::codex_control::CodexControlDecision::Deny,
                        None,
                    )
                    .map_err(RuntimeError::from)?;
                self.record_wire(
                    run_id,
                    host_epoch,
                    &PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity {
                        activity: ToolActivity {
                            tool_use_id: request.tool_use_id,
                            tool_name: request.tool_name,
                            phase: ToolPhase::Failed,
                            output_digest: None,
                        },
                    }),
                )?;
                return Ok(1);
            }
            crate::permission_preflight::PermissionPreflight::Ask => {}
        }
        let bytes = serde_json::to_vec(&normalized).map_err(|error| {
            RuntimeError::Ledger(gent_ports::LedgerError::Storage(error.to_string()))
        })?;
        ledger.save_pending_permission(&PermissionDecisionRequest {
            binding: PermissionDecisionBinding {
                decision_id: AgentChatDecisionId(request.request_key.clone()),
                request_idempotency_key: format!("codex:{}", request.request_key),
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
