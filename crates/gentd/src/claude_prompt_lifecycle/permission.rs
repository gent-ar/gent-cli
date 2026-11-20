use gent_drivers::{
    claude_control::{ClaudePermissionBehavior, ClaudePermissionRequest},
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
    PermissionRequestDigest, ToolPhase, TurnPhase,
};
use sha2::{Digest, Sha256};

use super::{ClaudePromptExecution, ClaudePromptLifecycle, missing_binding};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ClaudePermissionRecord {
    pub(super) facts: u16,
}

impl<L, D, R> ClaudePromptLifecycle<L, D, R>
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
    D: ClaudePromptExecution + Clone,
    R: PublicProviderResolver,
{
    pub(crate) fn respond_permission(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
    ) -> Result<(), RuntimeError> {
        self.respond_permission_with_input(run_id, request_id, behavior, persist_suggestions, None)
    }

    pub(crate) fn respond_permission_with_input(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
        updated_input: Option<serde_json::Value>,
    ) -> Result<(), RuntimeError> {
        if !self.active.contains_key(run_id) {
            return Err(gent_ports::PublicProviderRunError::NotActive.into());
        }
        self.runner
            .respond_claude_permission_with_input(
                run_id,
                request_id,
                behavior,
                persist_suggestions,
                updated_input,
            )
            .map_err(RuntimeError::from)
    }

    pub(super) fn record_permission_request(
        &mut self,
        run_id: &str,
        host_epoch: gent_types::HostEpoch,
        request: ClaudePermissionRequest,
    ) -> Result<ClaudePermissionRecord, RuntimeError> {
        let binding = self
            .active
            .get(run_id)
            .cloned()
            .ok_or_else(missing_binding)?;
        let ledger = self.runtime.ledger();
        let ingress = ledger.host_ingress()?;
        if ingress.epoch != host_epoch || ingress.mode != gent_ports::IngressMode::Open {
            return Err(missing_binding());
        }
        let policy = binding.permission.clone();
        let category = crate::permission_category::for_tool(&request.tool_name);
        let normalized = PermissionRequest::new(
            request.tool_name.clone(),
            category,
            request.operation_preview.clone(),
            request.child_id.clone(),
        );
        let config = ledger.current_conversation_config(&binding.prompt.message.conversation_id)?;
        let disallowed = config
            .as_ref()
            .map_or(&[][..], |config| config.disallowed_tools.as_slice());
        let mode = gent_runtime::AgentChatReadService::new(ledger.clone())
            .run_selection(&binding.prompt.message.conversation_id, run_id)?
            .mode;
        let automatic = match crate::permission_preflight::mode_gate(mode, &normalized) {
            crate::permission_preflight::ModeGate::Refuse(reason) => {
                Some(ClaudePermissionBehavior::Refuse(reason))
            }
            crate::permission_preflight::ModeGate::AskUser => None,
            crate::permission_preflight::ModeGate::Policy => {
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
                        Some(ClaudePermissionBehavior::Allow)
                    }
                    crate::permission_preflight::PermissionPreflight::Deny => {
                        Some(ClaudePermissionBehavior::Deny)
                    }
                    crate::permission_preflight::PermissionPreflight::Ask => None,
                }
            }
        };
        if let Some(behavior) = automatic {
            self.runner
                .respond_claude_permission_with_input(
                    run_id,
                    &request.request_id,
                    behavior,
                    false,
                    None,
                )
                .map_err(RuntimeError::from)?;
            self.record_wire(
                run_id,
                host_epoch,
                &PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity {
                    activity: gent_types::ToolActivity {
                        tool_use_id: request.tool_use_id,
                        tool_name: request.tool_name,
                        phase: if behavior == ClaudePermissionBehavior::Allow {
                            ToolPhase::Started
                        } else {
                            ToolPhase::Failed
                        },
                        output_digest: None,
                        input: request.operation_preview,
                        parent_tool_use_id: None,
                    },
                }),
            )?;
            return Ok(ClaudePermissionRecord { facts: 1 });
        }
        let digest = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&normalized).map_err(|error| {
                RuntimeError::Ledger(gent_ports::LedgerError::Storage(error.to_string()))
            })?)
        );
        let pending = PermissionDecisionRequest {
            binding: PermissionDecisionBinding {
                decision_id: AgentChatDecisionId(request.request_id.clone()),
                request_idempotency_key: format!("claude:{}", request.request_id),
                conversation_id: AgentChatConversationId(
                    binding.prompt.message.conversation_id.clone(),
                ),
                run_id: AgentChatRunId(run_id.into()),
                turn_id: binding.turn_id().into(),
                policy_id: policy.policy_id,
                policy_revision: policy.revision,
                host_epoch,
                request_digest_sha256: PermissionRequestDigest(digest),
            },
            request: normalized,
        };
        ledger.save_pending_permission(&pending)?;
        let background = request
            .child_id
            .as_ref()
            .is_some_and(|child| binding.background_children.contains(child));
        let root_waits = (binding.turns.place_request(background)
            == crate::provider_run_state::RequestPlacement::RootTurn)
            .then_some(PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::RootPhase {
                    phase: TurnPhase::WaitingPermission,
                },
            ));
        for fact in root_waits.into_iter().chain([PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::ToolActivity {
                activity: gent_types::ToolActivity {
                    tool_use_id: request.tool_use_id,
                    tool_name: request.tool_name,
                    phase: ToolPhase::WaitingPermission,
                    output_digest: None,
                    input: request.operation_preview,
                    parent_tool_use_id: None,
                },
            },
        )]) {
            self.record_wire(run_id, host_epoch, &fact)?;
        }
        Ok(ClaudePermissionRecord { facts: 2 })
    }
}
