use super::super::validation::invariant;
use super::super::{BoundSource, PrivateClaurstIngress};
use crate::claurst_permission_policy::ClaurstPermissionDecision;
use gent_ports::{AgentChatWorkspaceLedger, PendingPermissionLedger, PolicyLedger};

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
        + gent_ports::AgentChatConversationConfigLedger,
    L: gent_ports::AgentChatReadLedger,
    B: gent_ports::PrivateClaurstBridge,
{
    pub(in crate::private_claurst_ingress) async fn admit_permissions(
        &mut self,
        state: &BoundSource,
        permissions: &[gent_ports::ClaurstPermissionRequest],
        host_epoch: gent_types::HostEpoch,
    ) -> Result<(), gent_runtime::RuntimeError> {
        let source = &state.binding.source_id;
        let conversation_id = state
            .conversation_id
            .as_ref()
            .map(|id| id.0.clone())
            .unwrap_or_default();
        let run_id = &state.binding.run_id;
        self.reconsider_pending_permission(state, host_epoch)
            .await?;
        let mut queue = std::mem::take(&mut self.source_mut(source)?.queued_permissions);
        if let Some(next) =
            crate::permission_queue::next_ready(&self.ledger, &conversation_id, run_id, &mut queue)?
        {
            self.record_permission_request(state, &next, host_epoch)
                .await?;
        }
        for permission in permissions {
            if crate::permission_queue::waits_behind(
                &self.ledger,
                &conversation_id,
                run_id,
                &permission.request_id,
                &queue,
            )? {
                queue.push_back(permission.clone());
            } else {
                self.record_permission_request(state, permission, host_epoch)
                    .await?;
            }
        }
        self.source_mut(source)?.queued_permissions = queue;
        Ok(())
    }

    async fn reconsider_pending_permission(
        &mut self,
        state: &BoundSource,
        host_epoch: gent_types::HostEpoch,
    ) -> Result<(), gent_runtime::RuntimeError> {
        let (Some(conversation_id), Some(turn_id)) = (&state.conversation_id, &state.turn_id)
        else {
            return Ok(());
        };
        let run_id = &state.binding.run_id;
        let Some(pending) =
            crate::permission_queue::pending(&self.ledger, &conversation_id.0, run_id)?
                .filter(|pending| pending.binding.turn_id == *turn_id)
        else {
            return Ok(());
        };
        let policy = self.live_policy(&conversation_id.0, run_id)?;
        let source = self.source_mut(&state.binding.source_id)?;
        if !crate::permission_queue::reconsiders(
            &pending,
            &policy,
            &mut source.reconsidered_revision,
        ) {
            return Ok(());
        }
        let (reply, outcome) = match self
            .decide_permission(state, &policy, &pending.request, host_epoch)
            .await?
        {
            ClaurstPermissionDecision::Allow => (
                gent_ports::ClaurstPermissionReply::AllowOnce,
                gent_types::DecisionOutcome::Allowed,
            ),
            ClaurstPermissionDecision::Deny(_) => (
                gent_ports::ClaurstPermissionReply::Deny,
                gent_types::DecisionOutcome::Denied,
            ),
            ClaurstPermissionDecision::Ask => return Ok(()),
        };
        self.bridge
            .respond_permission(state.binding.clone(), &pending.binding.decision_id.0, reply)
            .await?;
        self.ledger
            .settle_delivered_permission(&pending.binding, None, outcome)?;
        Ok(())
    }

    pub(in crate::private_claurst_ingress) fn live_policy(
        &self,
        conversation_id: &str,
        run_id: &str,
    ) -> Result<gent_types::PolicyRecord, gent_runtime::RuntimeError> {
        let workspace = self
            .ledger
            .agent_chat_workspace_for_run(conversation_id, run_id)?;
        Ok(crate::permission_workspace::policy_for_conversation(
            &self.ledger,
            conversation_id,
            &workspace.workspace_id,
        )?)
    }

    pub(in crate::private_claurst_ingress) async fn decide_permission(
        &self,
        state: &BoundSource,
        policy: &gent_types::PolicyRecord,
        request: &gent_types::PermissionRequest,
        host_epoch: gent_types::HostEpoch,
    ) -> Result<ClaurstPermissionDecision, gent_runtime::RuntimeError> {
        let conversation_id = state
            .conversation_id
            .as_ref()
            .ok_or_else(|| invariant("private Claurst permission has no conversation"))?;
        let workspace = self
            .ledger
            .agent_chat_workspace_for_run(&conversation_id.0, &state.binding.run_id)?;
        let enforcement = self
            .bridge
            .permission_enforcement(state.binding.clone(), policy, host_epoch)
            .await;
        let owned_chat_tool = self
            .bridge
            .owns_chat_tool(&state.binding, &request.tool_name)
            .await;
        let mode = gent_runtime::AgentChatReadService::new(self.ledger.clone())
            .run_selection(&conversation_id.0, &state.binding.run_id)?
            .mode;
        Ok(crate::claurst_permission_policy::decide_with_enforcement(
            mode,
            policy,
            request,
            std::path::Path::new(&workspace.canonical_path),
            enforcement,
            owned_chat_tool,
        ))
    }

    fn source_mut(
        &mut self,
        source: &gent_ports::ClaurstSourceId,
    ) -> Result<&mut BoundSource, gent_runtime::RuntimeError> {
        self.sources
            .get_mut(source)
            .ok_or_else(|| invariant("private Claurst source is not bound"))
    }
}
