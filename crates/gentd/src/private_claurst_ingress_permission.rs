use gent_ports::{AgentChatWorkspaceLedger, PendingPermissionLedger, PolicyLedger};
use gent_types::{
    AgentChatConversationId, AgentChatDecisionId, AgentChatRunId, NormalizedLifecycleSignal,
    NormalizedSessionBatch, NormalizedSessionLifecycle, PermissionDecisionBinding,
    PermissionDecisionRequest, PermissionDenialReason, PermissionRequest, PermissionRequestDigest,
    ToolActivity, ToolPhase, TurnPhase,
};
use sha2::{Digest, Sha256};

use super::validation::{event_id, invariant};
use super::{BoundSource, PrivateClaurstIngress};
use crate::claurst_permission_policy::ClaurstPermissionDecision;

#[path = "private_claurst_ingress_permission_decision.rs"]
mod decision;

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
    L: gent_ports::AgentChatReadLedger,
    B: gent_ports::PrivateClaurstBridge,
{
    pub(super) async fn record_permission_request(
        &self,
        state: &BoundSource,
        request: &gent_ports::ClaurstPermissionRequest,
        host_epoch: gent_types::HostEpoch,
    ) -> Result<(), gent_runtime::RuntimeError> {
        let conversation_id = state
            .conversation_id
            .as_ref()
            .ok_or_else(|| invariant("private Claurst permission has no conversation"))?;
        let turn_id = state
            .turn_id
            .as_ref()
            .ok_or_else(|| invariant("private Claurst permission has no turn"))?;
        let workspace = self
            .ledger
            .agent_chat_workspace_for_run(&conversation_id.0, &state.binding.run_id)?;
        let policy =
            crate::permission_workspace::policy_for(&self.ledger, &workspace.workspace_id)?;
        let mode = gent_runtime::AgentChatReadService::new(self.ledger.clone())
            .run_selection(&conversation_id.0, &state.binding.run_id)?
            .mode;
        let normalized = PermissionRequest::new(
            request.tool_name.clone(),
            request.category,
            request.input.clone(),
            None,
        );
        match crate::claurst_permission_policy::decide(
            mode,
            &policy,
            &normalized,
            std::path::Path::new(&workspace.canonical_path),
        ) {
            ClaurstPermissionDecision::Allow => {
                self.bridge
                    .respond_permission(
                        state.binding.clone(),
                        &request.request_id,
                        gent_ports::ClaurstPermissionReply::AllowOnce,
                    )
                    .await?;
                return self.record_permission_activity(
                    state,
                    request,
                    host_epoch,
                    ToolPhase::Started,
                );
            }
            ClaurstPermissionDecision::Deny(reason) => {
                self.bridge
                    .respond_permission(
                        state.binding.clone(),
                        &request.request_id,
                        gent_ports::ClaurstPermissionReply::Deny,
                    )
                    .await?;
                return self.record_permission_lifecycle(
                    state,
                    &format!("permission-{}-denied", request.request_id),
                    NormalizedSessionLifecycle::Signal {
                        signal: tool_signal(request, ToolPhase::Failed),
                    },
                    host_epoch,
                    reason.map(PermissionDenialReason::notice),
                );
            }
            ClaurstPermissionDecision::Ask => {}
        }
        let digest = serde_json::to_vec(&normalized)
            .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
            .map_err(|error| {
                gent_runtime::RuntimeError::Ledger(gent_ports::LedgerError::Storage(
                    error.to_string(),
                ))
            })?;
        let binding = PermissionDecisionBinding {
            decision_id: AgentChatDecisionId(request.request_id.clone()),
            request_idempotency_key: format!(
                "claurst:{}:{}",
                state.binding.source_id.0, request.request_id
            ),
            conversation_id: AgentChatConversationId(conversation_id.0.clone()),
            run_id: AgentChatRunId(state.binding.run_id.clone()),
            turn_id: turn_id.clone(),
            policy_id: policy.policy_id,
            policy_revision: policy.revision,
            host_epoch,
            request_digest_sha256: PermissionRequestDigest(digest),
        };
        self.ledger
            .save_pending_permission(&PermissionDecisionRequest {
                binding,
                request: normalized,
            })?;
        self.record_permission_wait(state, request, host_epoch)
    }
}

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
    fn record_permission_activity(
        &self,
        state: &BoundSource,
        request: &gent_ports::ClaurstPermissionRequest,
        host_epoch: gent_types::HostEpoch,
        phase: ToolPhase,
    ) -> Result<(), gent_runtime::RuntimeError> {
        self.record_permission_lifecycle(
            state,
            &format!("permission-{}-{phase:?}", request.request_id),
            NormalizedSessionLifecycle::Signal {
                signal: tool_signal(request, phase),
            },
            host_epoch,
            None,
        )
    }

    fn record_permission_wait(
        &self,
        state: &BoundSource,
        request: &gent_ports::ClaurstPermissionRequest,
        host_epoch: gent_types::HostEpoch,
    ) -> Result<(), gent_runtime::RuntimeError> {
        for (suffix, signal) in [
            (
                "root",
                NormalizedLifecycleSignal::RootPhase {
                    phase: TurnPhase::WaitingPermission,
                },
            ),
            ("tool", tool_signal(request, ToolPhase::WaitingPermission)),
        ] {
            self.record_permission_lifecycle(
                state,
                &format!("permission-{}-{suffix}", request.request_id),
                NormalizedSessionLifecycle::Signal { signal },
                host_epoch,
                None,
            )?;
        }
        Ok(())
    }

    fn record_permission_lifecycle(
        &self,
        state: &BoundSource,
        suffix: &str,
        lifecycle: NormalizedSessionLifecycle,
        host_epoch: gent_types::HostEpoch,
        notice: Option<&str>,
    ) -> Result<(), gent_runtime::RuntimeError> {
        let conversation_id = state.conversation_id.as_ref().expect("validated above");
        let turn_id = state.turn_id.as_ref().expect("validated above");
        let activity = crate::public_driver_runtime::session::activity_for_lifecycle(
            &conversation_id.0,
            &state.binding.run_id,
            turn_id,
            host_epoch,
            &lifecycle,
        );
        self.ledger
            .append_normalized_session_batch(&NormalizedSessionBatch {
                coordinator_id: self.coordinator_id.clone(),
                conversation_id: conversation_id.0.clone(),
                run_id: state.binding.run_id.clone(),
                turn_id: turn_id.clone(),
                host_epoch,
                lifecycle_event_id: event_id(&state.binding.source_id, suffix),
                lifecycle,
                transcript: notice.map(|text| gent_types::NormalizedTranscriptAppend {
                    event_id: event_id(&state.binding.source_id, &format!("{suffix}-notice")),
                    turn_id: turn_id.clone(),
                    run_id: state.binding.run_id.clone(),
                    kind: gent_types::NormalizedTranscriptKind::Notice,
                    text: text.into(),
                    is_partial: false,
                }),
                activity_event_id: activity
                    .as_ref()
                    .map(|_| event_id(&state.binding.source_id, &format!("{suffix}-activity"))),
                activity,
            })
            .map(|_| ())
            .map_err(Into::into)
    }
}

fn tool_signal(
    request: &gent_ports::ClaurstPermissionRequest,
    phase: ToolPhase,
) -> NormalizedLifecycleSignal {
    NormalizedLifecycleSignal::ToolActivity {
        activity: ToolActivity {
            tool_use_id: request.tool_use_id.clone(),
            tool_name: request.tool_name.clone(),
            phase,
            output_digest: None,
            parent_tool_use_id: None,
        },
    }
}

#[cfg(test)]
#[path = "private_claurst_ingress_permission_tests.rs"]
mod tests;
