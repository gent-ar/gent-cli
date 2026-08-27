use gent_ports::{AgentChatWorkspaceLedger, PendingPermissionLedger, PolicyLedger};
use gent_types::{
    AgentChatConversationId, AgentChatDecisionId, AgentChatRunId, Command, Event,
    NormalizedLifecycleSignal, NormalizedProviderEvent, NormalizedSessionBatch,
    NormalizedSessionLifecycle, PermissionDecisionBinding, PermissionDecisionRequest,
    PermissionDecisionResponse, PermissionDecisionResponseKind, PermissionRequest,
    PermissionRequestDigest, PolicyRecord, PolicyScope, Receipt, ReceiptId, ReceiptStatus,
    ToolActivity, ToolPhase, TurnPhase,
};
use sha2::{Digest, Sha256};

use super::validation::{event_id, invariant};
use super::{BoundSource, PrivateClaurstIngress};

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
        let normalized = PermissionRequest {
            tool_name: request.tool_name.clone(),
            category: request.category,
            input: None,
        };
        match crate::permission_preflight::evaluate(&policy, &normalized) {
            crate::permission_preflight::PermissionPreflight::Allow => {
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
            crate::permission_preflight::PermissionPreflight::Deny => {
                self.bridge
                    .respond_permission(
                        state.binding.clone(),
                        &request.request_id,
                        gent_ports::ClaurstPermissionReply::Deny,
                    )
                    .await?;
                return self.record_permission_activity(
                    state,
                    request,
                    host_epoch,
                    ToolPhase::Failed,
                );
            }
            crate::permission_preflight::PermissionPreflight::Ask => {}
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
                signal: NormalizedLifecycleSignal::ToolActivity {
                    activity: ToolActivity {
                        tool_use_id: request.tool_use_id.clone(),
                        tool_name: request.tool_name.clone(),
                        phase,
                        output_digest: None,
                    },
                },
            },
            host_epoch,
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
            (
                "tool",
                NormalizedLifecycleSignal::ToolActivity {
                    activity: ToolActivity {
                        tool_use_id: request.tool_use_id.clone(),
                        tool_name: request.tool_name.clone(),
                        phase: ToolPhase::WaitingPermission,
                        output_digest: None,
                    },
                },
            ),
        ] {
            self.record_permission_lifecycle(
                state,
                &format!("permission-{}-{suffix}", request.request_id),
                NormalizedSessionLifecycle::Signal { signal },
                host_epoch,
            )?;
        }
        Ok(())
    }

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
        )?;
        Ok(())
    }

    fn record_permission_lifecycle(
        &self,
        state: &BoundSource,
        suffix: &str,
        lifecycle: NormalizedSessionLifecycle,
        host_epoch: gent_types::HostEpoch,
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
                transcript: None,
                activity_event_id: activity
                    .as_ref()
                    .map(|_| event_id(&state.binding.source_id, &format!("{suffix}-activity"))),
                activity,
            })
            .map(|_| ())
            .map_err(Into::into)
    }

    pub(crate) async fn respond_permission_with_receipt(
        &self,
        response: PermissionDecisionResponse,
        receipt_id: ReceiptId,
    ) -> Result<Receipt, gent_runtime::RuntimeError> {
        let command = Command {
            receipt_id: receipt_id.clone(),
            idempotency_key: response.binding.request_idempotency_key.clone(),
            host_epoch: response.binding.host_epoch,
            kind: "agentChatPermissionDecision".into(),
            payload: serde_json::to_value(&response).map_err(|error| {
                gent_runtime::RuntimeError::Ledger(gent_ports::LedgerError::Storage(
                    error.to_string(),
                ))
            })?,
        };
        let accepted = Event {
            cursor: 0,
            event_id: format!(
                "permission-decision-accepted:{}",
                response.binding.decision_id.0
            ),
            receipt_id: receipt_id.clone(),
            host_epoch: response.binding.host_epoch,
            kind: "agentChatPermissionDecisionAccepted".into(),
            payload: command.payload.clone(),
        };
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
        let status_name = status_name(&status);
        self.ledger
            .settle_receipt(
                &command.idempotency_key,
                status,
                &Event {
                    cursor: 0,
                    event_id: format!("permission-decision-{status_name}:{}", command.receipt_id.0),
                    receipt_id: command.receipt_id.clone(),
                    host_epoch: command.host_epoch,
                    kind: "agentChatPermissionDecisionTerminal".into(),
                    payload: serde_json::json!({"status": status_name}),
                },
            )
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

fn status_name(status: &ReceiptStatus) -> &'static str {
    match status {
        ReceiptStatus::Settled => "settled",
        ReceiptStatus::Unprovable => "unprovable",
        ReceiptStatus::Rejected => "rejected",
        ReceiptStatus::Accepted => "accepted",
    }
}
