use super::{
    ActiveRuntime, ClaurstBridgeHandle, ClaurstStandaloneOwner, HttpLlamaServerReadiness, PathBuf,
    RuntimeIdentity, StandaloneClaurstRuntimeFactory, SystemPrivateSettingsStore,
};

impl StandaloneClaurstRuntimeFactory {
    pub(super) async fn launch_runtime(
        &self,
        identity: RuntimeIdentity,
        request: crate::claurst_local_runtime::ClaurstLocalRuntimeRequest,
        workspace: PathBuf,
        mcp_servers: Vec<serde_json::Value>,
    ) -> Result<(), String> {
        let readiness = self.readiness.clone();
        let launcher = crate::claurst_sandbox_launcher::RoutedClaurstStandaloneLauncher::new(
            self.ledger.clone(),
            identity.permission_binding().cloned(),
        )?
        .with_mcp_control(self.mcp_control.clone());
        let model_id = identity.model.model_id.clone();
        let runtime = tokio::task::spawn_blocking(move || {
            ClaurstStandaloneOwner::new(
                readiness,
                SystemPrivateSettingsStore,
                launcher,
                HttpLlamaServerReadiness::default(),
            )
            .start_with_mcp(&model_id, request, &workspace, mcp_servers)
        })
        .await
        .map_err(|_| "local Claurst startup worker stopped unexpectedly".to_owned())?
        .map_err(|error| error.to_string())?;
        let bridge = ClaurstBridgeHandle::new(runtime.bridge());
        let mut active = self.active.lock().await;
        if active.is_some() {
            drop(bridge);
            let _ = tokio::task::spawn_blocking(move || runtime.shutdown()).await;
            return Err("local Claurst runtime changed while it was starting".into());
        }
        *active = Some(ActiveRuntime {
            identity,
            bridge,
            runtime,
        });
        Ok(())
    }
}

impl StandaloneClaurstRuntimeFactory {
    pub(crate) fn has_active_runtime(&self) -> bool {
        self.active
            .try_lock()
            .map_or(true, |active| active.is_some())
    }
}
