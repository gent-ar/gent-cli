use std::sync::Arc;

use async_trait::async_trait;
use gent_ports::ClaurstPromptAttachment;
use gent_runtime::conversation_summary_scheduler::ConversationSummaryScheduler;
use gent_store::SqliteLedger;
use gent_types::{AgentChatPromptSaved, AttachmentMetadata};
use sha2::{Digest, Sha256};

use crate::{
    claurst_acp_bridge::ClaurstBridgeHandle,
    claurst_local_readiness::ClaurstLocalReadinessService,
    claurst_local_runtime::ClaurstLocalRuntimeRequest,
    claurst_metadata_summary::ClaurstMetadataSummaryRunner,
    claurst_runtime_factory::{ClaurstRuntimeFactory, ContextSummarizer},
    claurst_sandbox_stdio::ClaurstOwnedAcpStdio,
    claurst_standalone_owner::ClaurstStandaloneRuntime,
};

#[path = "standalone_claurst_runtime_factory_launch.rs"]
mod launch;
use crate::standalone_claurst_runtime_identity::RuntimeIdentity;

type SystemRuntime = ClaurstStandaloneRuntime<
    crate::claurst_sandbox_launcher::model_launch::RoutedModelProcess,
    ClaurstOwnedAcpStdio,
>;
type SystemBridge = ClaurstBridgeHandle<ClaurstOwnedAcpStdio>;

struct ActiveRuntime {
    identity: RuntimeIdentity,
    run_id: String,
    used: std::time::Instant,
    idle_since: Option<std::time::Instant>,
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
    mcp_control: Option<crate::ordinary_lifecycle_control::OrdinaryLifecycleControl>,
    launching: tokio::sync::Mutex<()>,
    active: tokio::sync::Mutex<std::collections::BTreeMap<String, ActiveRuntime>>,
}

#[derive(Clone, Debug)]
pub(crate) struct StandaloneClaurstBridge(Arc<StandaloneClaurstRuntimeFactory>);

impl StandaloneClaurstRuntimeFactory {
    pub(crate) async fn stop_conversation(
        &self,
        conversation_id: &str,
        background: gent_types::ToolPhase,
    ) -> Result<(), String> {
        let previous = self.active.lock().await.remove(conversation_id);
        self.end_background(conversation_id, background)?;
        if let Some(previous) = previous {
            previous
                .shut_down("could not stop local Claurst runtime")
                .await?;
        }
        Ok(())
    }

    pub(super) fn end_background(
        &self,
        conversation_id: &str,
        background: gent_types::ToolPhase,
    ) -> Result<(), String> {
        gent_ports::RunLifecycleFactLedger::end_background_tools(
            &self.ledger,
            conversation_id,
            background,
        )
        .map_err(|error| error.to_string())
    }

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
            mcp_control: None,
            launching: tokio::sync::Mutex::new(()),
            active: tokio::sync::Mutex::new(std::collections::BTreeMap::new()),
        }
    }

    pub(crate) fn with_mcp_control(
        mut self,
        control: crate::ordinary_lifecycle_control::OrdinaryLifecycleControl,
    ) -> Self {
        self.mcp_control = Some(control);
        self
    }
    #[must_use]
    pub(crate) fn bridge(self: &Arc<Self>) -> StandaloneClaurstBridge {
        StandaloneClaurstBridge(Arc::clone(self))
    }
}

impl StandaloneClaurstBridge {
    pub(crate) fn runtime(&self) -> &StandaloneClaurstRuntimeFactory {
        &self.0
    }
}

#[async_trait]
impl ClaurstRuntimeFactory for Arc<StandaloneClaurstRuntimeFactory> {
    async fn ensure_for_prompt(
        &self,
        saved: &AgentChatPromptSaved,
        busy: &[String],
    ) -> Result<(), String> {
        let policy = self
            .start_selected(
                &saved.message.conversation_id,
                &saved.run_id.0,
                &saved.tool_source_ids,
                busy,
            )
            .await?;
        self.run_bridge(&saved.run_id.0)
            .await
            .and_then(|bridge| bridge.bind_turn_policy(policy))
            .map_err(|error| error.to_string())
    }

    async fn warm(
        &self,
        conversation_id: &str,
        run_id: &str,
        busy: &[String],
    ) -> Result<(), String> {
        self.start_selected(conversation_id, run_id, &[], busy)
            .await
            .map(|_| ())
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

    async fn reap_idle(&self, now: std::time::Instant, busy: &[String]) -> Result<(), String> {
        self.reap_idle_runtimes(now, busy).await
    }

    async fn after_prompt_failed(
        &self,
        conversation_id: &str,
        background: gent_types::ToolPhase,
    ) -> Result<(), String> {
        self.stop_conversation(conversation_id, background).await
    }

    async fn context_summarizer(
        &self,
        conversation_id: &str,
    ) -> Option<Arc<dyn ContextSummarizer>> {
        self.active
            .lock()
            .await
            .get(conversation_id)
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

#[path = "standalone_claurst_runtime_factory_bridge.rs"]
mod bridge_port;

#[cfg(test)]
#[path = "standalone_claurst_runtime_factory_tests.rs"]
mod tests;
