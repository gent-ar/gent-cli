use async_trait::async_trait;
use gent_ports::{
    ClaurstDrainBatch, ClaurstDrainRequest, ClaurstPermissionReply, ClaurstSessionBinding,
    ClaurstStartRequest, ClaurstSubmitRequest, PortError, PrivateClaurstBridge,
};

use super::StandaloneClaurstBridge;

#[async_trait]
impl PrivateClaurstBridge for StandaloneClaurstBridge {
    async fn start(
        &self,
        request: ClaurstStartRequest,
    ) -> Result<ClaurstSessionBinding, PortError> {
        self.0
            .run_bridge(&request.run_id)
            .await?
            .start(request)
            .await
    }

    async fn bind_session(&self, binding: ClaurstSessionBinding) -> Result<(), PortError> {
        self.0
            .run_bridge(&binding.run_id)
            .await?
            .bind_session(binding)
            .await
    }

    async fn submit(&self, request: ClaurstSubmitRequest) -> Result<(), PortError> {
        self.0
            .run_bridge(&request.binding.run_id)
            .await?
            .submit(request)
            .await
    }

    async fn cancel(&self, binding: ClaurstSessionBinding) -> Result<(), PortError> {
        self.0
            .run_bridge(&binding.run_id)
            .await?
            .cancel(binding)
            .await
    }

    async fn drain(&self, request: ClaurstDrainRequest) -> Result<ClaurstDrainBatch, PortError> {
        self.0
            .run_bridge(&request.run_id)
            .await?
            .drain(request)
            .await
    }

    async fn permission_enforcement(
        &self,
        binding: ClaurstSessionBinding,
        policy: &gent_types::PolicyRecord,
        epoch: gent_types::HostEpoch,
    ) -> gent_types::SandboxEnforcement {
        let Ok(bridge) = self.0.run_bridge(&binding.run_id).await else {
            return gent_types::SandboxEnforcement::Unavailable;
        };
        bridge.permission_enforcement(binding, policy, epoch).await
    }

    async fn owns_chat_tool(&self, binding: &ClaurstSessionBinding, tool_name: &str) -> bool {
        let Ok(bridge) = self.0.run_bridge(&binding.run_id).await else {
            return false;
        };
        bridge.owns_chat_tool(binding, tool_name).await
    }

    async fn runs_background_commands(&self, binding: &ClaurstSessionBinding) -> bool {
        let Ok(bridge) = self.0.run_bridge(&binding.run_id).await else {
            return false;
        };
        bridge.runs_background_commands(binding).await
    }

    async fn respond_permission(
        &self,
        binding: ClaurstSessionBinding,
        request_id: &str,
        reply: ClaurstPermissionReply,
    ) -> Result<(), PortError> {
        self.0
            .run_bridge(&binding.run_id)
            .await?
            .respond_permission(binding, request_id, reply)
            .await
    }
}
