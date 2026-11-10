use super::super::validation::invariant;
use super::super::{BoundSource, PrivateClaurstIngress};
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

    pub(in crate::private_claurst_ingress) async fn deny_queued_permissions(
        &mut self,
        source: &gent_ports::ClaurstSourceId,
    ) -> Result<(), gent_runtime::RuntimeError> {
        let state = self.source_mut(source)?;
        let queued = std::mem::take(&mut state.queued_permissions);
        let binding = state.binding.clone();
        for request in queued {
            let _ = self
                .bridge
                .respond_permission(
                    binding.clone(),
                    &request.request_id,
                    gent_ports::ClaurstPermissionReply::Deny,
                )
                .await;
        }
        Ok(())
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
