use std::sync::Arc;

use async_trait::async_trait;
use gent_ports::{
    ClaurstDrainBatch, ClaurstDrainRequest, ClaurstPermissionReply, ClaurstPromptAttachment,
    ClaurstSessionBinding, ClaurstStartRequest, ClaurstSubmitRequest, PortError,
    PrivateClaurstBridge,
};
use gent_runtime::conversation_summary_scheduler::ConversationSummaryScheduler;
use gent_store::SqliteLedger;
use gent_types::{AgentChatPromptSaved, AttachmentMetadata};
use sha2::{Digest, Sha256};

use crate::{
    claurst_acp_bridge::ClaurstBridgeHandle,
    claurst_local_readiness::ClaurstLocalReadinessService,
    claurst_local_runtime::ClaurstLocalRuntimeRequest,
    claurst_local_runtime_owner::{SystemClaurstAcpStdio, SystemLocalRuntimeProcess},
    claurst_metadata_summary::ClaurstMetadataSummaryRunner,
    claurst_runtime_factory::{ClaurstRuntimeFactory, ContextSummarizer},
    claurst_standalone_owner::ClaurstStandaloneRuntime,
};

#[path = "standalone_claurst_runtime_factory_launch.rs"]
mod launch;
use crate::standalone_claurst_runtime_identity::RuntimeIdentity;

type SystemRuntime = ClaurstStandaloneRuntime<SystemLocalRuntimeProcess, SystemClaurstAcpStdio>;
type SystemBridge = ClaurstBridgeHandle<SystemClaurstAcpStdio>;

struct ActiveRuntime {
    identity: RuntimeIdentity,
    bridge: SystemBridge,
    runtime: SystemRuntime,
}

impl ActiveRuntime {
    async fn shut_down(self, failure: &str) -> Result<(), String> {
        let Self {
            bridge, runtime, ..
        } = self;
        drop(bridge);
        tokio::task::spawn_blocking(move || runtime.shutdown())
            .await
            .map_err(|_| "local Claurst shutdown worker stopped unexpectedly".to_owned())?
            .map_err(|error| format!("{failure}: {error}"))
    }
}

impl std::fmt::Debug for ActiveRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ActiveRuntime(..)")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StandaloneClaurstRuntimeConfig {
    pub(crate) request: ClaurstLocalRuntimeRequest,
    pub(crate) mcp_config: Option<crate::standalone_mcp_config::StandaloneMcpConfig>,
}

#[derive(Debug)]
pub(crate) struct StandaloneClaurstRuntimeFactory {
    ledger: SqliteLedger,
    readiness: ClaurstLocalReadinessService,
    config: Option<StandaloneClaurstRuntimeConfig>,
    active: tokio::sync::Mutex<Option<ActiveRuntime>>,
}

#[derive(Clone, Debug)]
pub(crate) struct StandaloneClaurstBridge(Arc<StandaloneClaurstRuntimeFactory>);

impl StandaloneClaurstRuntimeFactory {
    #[must_use]
    pub(crate) fn new(
        ledger: SqliteLedger,
        models: crate::standalone_authority_composition::StandaloneClaurstModels,
        config: Option<StandaloneClaurstRuntimeConfig>,
    ) -> Self {
        Self {
            ledger,
            readiness: ClaurstLocalReadinessService::new(models.provisioner),
            config,
            active: tokio::sync::Mutex::new(None),
        }
    }

    #[must_use]
    pub(crate) fn bridge(self: &Arc<Self>) -> StandaloneClaurstBridge {
        StandaloneClaurstBridge(Arc::clone(self))
    }

    async fn active_bridge(&self) -> Result<SystemBridge, PortError> {
        self.active
            .lock()
            .await
            .as_ref()
            .map(|active| active.bridge.clone())
            .ok_or_else(|| PortError::Unavailable("local Claurst runtime is not ready".into()))
    }

    pub(crate) async fn summary_bridge(&self, model_id: &str) -> Result<SystemBridge, PortError> {
        let active = self.active.lock().await;
        let runtime = active
            .as_ref()
            .ok_or_else(|| PortError::Unavailable("local Claurst runtime is not ready".into()))?;
        if runtime.identity.model.model_id != model_id {
            return Err(PortError::Unavailable(
                "selected Claurst model is not the active local model".into(),
            ));
        }
        if !runtime.bridge.is_idle().map_err(PortError::Unavailable)? {
            return Err(PortError::Unavailable(
                "Claurst summary waits for the interactive prompt to become idle".into(),
            ));
        }
        Ok(runtime.bridge.clone())
    }

    async fn stop_active(&self) -> Result<(), String> {
        let previous = self.active.lock().await.take();
        if let Some(previous) = previous {
            previous
                .shut_down("could not stop local Claurst runtime")
                .await?;
        }
        Ok(())
    }
}

impl StandaloneClaurstBridge {
    pub(crate) async fn summary_bridge(&self, model_id: &str) -> Result<SystemBridge, PortError> {
        self.0.summary_bridge(model_id).await
    }
}

#[async_trait]
impl ClaurstRuntimeFactory for Arc<StandaloneClaurstRuntimeFactory> {
    async fn ensure_for_prompt(&self, saved: &AgentChatPromptSaved) -> Result<(), String> {
        self.start_selected(saved).await
    }

    async fn after_prompt_settled(&self, conversation_id: &str) -> Result<(), String> {
        if let Err(error) =
            ConversationSummaryScheduler::new(self.ledger.clone(), ClaurstMetadataSummaryRunner)
                .schedule(conversation_id)
        {
            eprintln!("Claurst summary failed: {error}");
        }
        Ok(())
    }

    async fn after_prompt_failed(&self, _: &str) -> Result<(), String> {
        self.stop_active().await
    }

    async fn context_summarizer(&self) -> Option<Arc<dyn ContextSummarizer>> {
        self.active
            .lock()
            .await
            .as_ref()
            .map(|active| active.runtime.summarizer() as Arc<dyn ContextSummarizer>)
    }

    async fn prompt_attachments(
        &self,
        metadata: &[AttachmentMetadata],
    ) -> Result<Vec<ClaurstPromptAttachment>, String> {
        if metadata.is_empty() {
            return Ok(Vec::new());
        }
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| "Claurst local runtime is not configured".to_owned())?;
        let data_dir = config
            .request
            .claurst_home
            .parent()
            .ok_or_else(|| "Claurst local runtime has no data directory".to_owned())?;
        let blobs = gent_store::FileAttachmentBlobs::open(data_dir.join("attachments"))
            .map_err(|error| error.to_string())?;
        metadata
            .iter()
            .map(|attachment| {
                let bytes = gent_ports::AttachmentBlobStore::read_attachment_blob(
                    &blobs,
                    &attachment.storage_key,
                )
                .map_err(|error| error.to_string())?;
                let digest = format!("{:x}", Sha256::digest(&bytes));
                if bytes.len() as u64 != attachment.byte_len || digest != attachment.digest_sha256 {
                    return Err("Claurst attachment content failed its durable digest check".into());
                }
                Ok(ClaurstPromptAttachment {
                    display_name: attachment.display_name.clone(),
                    media_type: attachment.media_type.clone(),
                    bytes,
                })
            })
            .collect()
    }
}

#[async_trait]
impl PrivateClaurstBridge for StandaloneClaurstBridge {
    async fn start(
        &self,
        request: ClaurstStartRequest,
    ) -> Result<ClaurstSessionBinding, PortError> {
        self.0.active_bridge().await?.start(request).await
    }

    async fn bind_session(&self, binding: ClaurstSessionBinding) -> Result<(), PortError> {
        self.0.active_bridge().await?.bind_session(binding).await
    }

    async fn submit(&self, request: ClaurstSubmitRequest) -> Result<(), PortError> {
        self.0.active_bridge().await?.submit(request).await
    }

    async fn cancel(&self, binding: ClaurstSessionBinding) -> Result<(), PortError> {
        self.0.active_bridge().await?.cancel(binding).await
    }

    async fn drain(&self, request: ClaurstDrainRequest) -> Result<ClaurstDrainBatch, PortError> {
        self.0.active_bridge().await?.drain(request).await
    }

    async fn respond_permission(
        &self,
        binding: ClaurstSessionBinding,
        request_id: &str,
        reply: ClaurstPermissionReply,
    ) -> Result<(), PortError> {
        self.0
            .active_bridge()
            .await?
            .respond_permission(binding, request_id, reply)
            .await
    }
}

#[cfg(test)]
#[path = "standalone_claurst_runtime_factory_tests.rs"]
mod tests;
