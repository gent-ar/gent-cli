use std::path::PathBuf;

use super::StandaloneAuthorityRuntime;
use crate::standalone_claurst_runtime_factory::StandaloneClaurstBridge;

type SystemClaurstRuntime = crate::claurst_standalone_owner::ClaurstStandaloneRuntime<
    crate::claurst_local_runtime_owner::SystemLocalRuntimeProcess,
    crate::claurst_local_runtime_owner::SystemClaurstAcpStdio,
>;

pub(super) struct RetainedClaurstRuntime(SystemClaurstRuntime);

impl std::fmt::Debug for RetainedClaurstRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RetainedClaurstRuntime(..)")
    }
}

impl StandaloneAuthorityRuntime {
    pub(crate) async fn attach_claurst_bridge<B>(&self, bridge: B) -> Result<(), String>
    where
        B: gent_ports::PrivateClaurstBridge + std::fmt::Debug + Send + 'static,
    {
        self.prompt_ingress
            .attach_async_claurst(Box::new(
                crate::claurst_prompt_lifecycle::ClaurstPromptLifecycle::new(
                    self.ledger.clone(),
                    bridge,
                    self.coordinator_id.clone(),
                    self.host_epoch,
                ),
            ))
            .await
    }

    pub(crate) async fn attach_lazy_claurst_runtime(
        &self,
        config: Option<crate::standalone_claurst_runtime_factory::StandaloneClaurstRuntimeConfig>,
    ) -> Result<(), String> {
        let factory = std::sync::Arc::new(
            crate::standalone_claurst_runtime_factory::StandaloneClaurstRuntimeFactory::new(
                self.ledger.clone(),
                self.claurst_models.clone(),
                config,
            ),
        );
        if let Ok(mut bridge) = self.claurst_side_question_bridge.lock() {
            *bridge = Some(factory.bridge());
        }
        self.prompt_ingress
            .attach_async_claurst(Box::new(
                crate::claurst_prompt_lifecycle::ClaurstPromptLifecycle::new_with_runtime(
                    self.ledger.clone(),
                    factory.bridge(),
                    factory,
                    self.coordinator_id.clone(),
                    self.host_epoch,
                ),
            ))
            .await
    }

    /// The bridge side questions use to run on the currently attached local Claurst runtime,
    /// once `attach_lazy_claurst_runtime` has run. `None` before then, or when Claurst is
    /// attached through `attach_claurst_bridge`/`start_local_claurst` instead.
    #[must_use]
    pub(crate) fn claurst_side_question_bridge(&self) -> Option<StandaloneClaurstBridge> {
        self.claurst_side_question_bridge
            .lock()
            .ok()
            .and_then(|bridge| bridge.clone())
    }

    pub(crate) async fn start_local_claurst(
        &self,
        model_id: String,
        request: crate::claurst_local_runtime::ClaurstLocalRuntimeRequest,
        workspace: PathBuf,
    ) -> Result<(), String> {
        if self
            .claurst_runtime
            .lock()
            .map_err(|_| "standalone Claurst runtime lock is unavailable".to_owned())?
            .is_some()
        {
            return Err("a standalone Claurst runtime is already active".into());
        }
        let readiness = crate::claurst_local_readiness::ClaurstLocalReadinessService::new(
            self.claurst_models.provisioner.clone(),
        );
        let runtime = tokio::task::spawn_blocking(move || {
            crate::claurst_standalone_owner::ClaurstStandaloneOwner::new(
                readiness,
                crate::claurst_local_runtime_owner::SystemPrivateSettingsStore,
                crate::claurst_local_runtime_owner::SystemClaurstStandaloneLauncher,
                crate::claurst_local_runtime_owner::HttpLlamaServerReadiness::default(),
            )
            .start(&model_id, request, &workspace)
        })
        .await
        .map_err(|_| "local Claurst startup worker stopped unexpectedly".to_owned())?
        .map_err(|error| error.to_string())?;
        let bridge = crate::claurst_acp_bridge::ClaurstBridgeHandle::new(runtime.bridge());
        if let Err(error) = self.attach_claurst_bridge(bridge).await {
            let _ = tokio::task::spawn_blocking(move || runtime.shutdown()).await;
            return Err(error);
        }
        let rejected = {
            let mut retained = self
                .claurst_runtime
                .lock()
                .map_err(|_| "standalone Claurst runtime lock is unavailable".to_owned())?;
            if retained.is_some() {
                Some(runtime)
            } else {
                *retained = Some(RetainedClaurstRuntime(runtime));
                None
            }
        };
        if let Some(runtime) = rejected {
            let _ = tokio::task::spawn_blocking(move || runtime.shutdown()).await;
            return Err("a standalone Claurst runtime is already active".into());
        }
        Ok(())
    }
}
