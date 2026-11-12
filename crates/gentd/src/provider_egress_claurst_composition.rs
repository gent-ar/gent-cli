use super::{
    Arc, AtomicBool, Coordinator, EgressError, Path, ProviderEgressBroker, ProviderGatewayLease,
    ReviewedProviderEgress, RunFence, RunVersionLock, SandboxPrivateState, SqliteLedger,
};
impl ReviewedProviderEgress {
    pub(crate) fn contained_model(
        endpoint: crate::contained_model::ModelEndpoint,
    ) -> Result<Self, EgressError> {
        use sha2::{Digest, Sha256};
        if !endpoint.current() {
            return Err(EgressError::Configuration);
        }
        Ok(Self {
            api: super::super::ProviderApi::Claurst,
            target: super::super::ProviderTarget::ContainedModel(endpoint.clone()),
            credentials: reqwest::header::HeaderMap::new(),
            policy_digest: format!("{:x}", Sha256::digest(endpoint.identity().as_bytes())),
        })
    }
}

#[path = "claurst_mcp_gateway_settings.rs"]
mod claurst_mcp_settings;

impl ProviderGatewayLease {
    pub(crate) fn claurst_contained(
        ledger: SqliteLedger,
        binding: &gent_types::SandboxPermissionBinding,
        lock: RunVersionLock,
        root: &Path,
        endpoint: crate::contained_model::ModelEndpoint,
    ) -> Result<Arc<Self>, EgressError> {
        let coordinator = Coordinator::new(ledger.clone(), gent_types::CapabilitySet::default());
        let fence = Arc::new(RunFence::capture(ledger, coordinator, binding, lock)?);
        if fence.lock.provider != "claurst"
            || !endpoint.matches_authority(&fence.run_id, &fence.lock)
            || !endpoint.current()
        {
            return Err(EgressError::Configuration);
        }
        let private_root = tempfile::Builder::new()
            .prefix("claurst-state-")
            .tempdir_in(root)
            .map_err(|_| EgressError::Configuration)?;
        let canonical = private_root
            .path()
            .canonicalize()
            .map_err(|_| EgressError::Configuration)?;
        let private_state = SandboxPrivateState::new(fence.run_id.clone(), canonical)
            .map_err(|_| EgressError::Configuration)?;
        for directory in private_state.directories() {
            std::fs::create_dir(directory).map_err(|_| EgressError::Configuration)?;
        }
        let policy = ReviewedProviderEgress::contained_model(endpoint.clone())?;
        let current = fence.clone();
        let model = endpoint.clone();
        let broker = ProviderEgressBroker::start_checked(
            policy,
            Arc::new(move || current.current() && model.current()),
        )?;
        Ok(Arc::new(Self {
            fence,
            broker,
            private_state,
            _private_root: private_root,
            retained_resources: std::sync::Mutex::default(),
            revoked: AtomicBool::new(false),
            contained_model: Some(endpoint),
            #[cfg(any(windows, target_os = "linux"))]
            mcp: std::sync::Mutex::default(),
            #[cfg(target_os = "linux")]
            mcp_deployment: std::sync::Mutex::default(),
        }))
    }

    pub(crate) fn materialize_claurst_settings(&self, contents: &str) -> Result<(), EgressError> {
        #[cfg(windows)]
        let clients = {
            use sha2::{Digest, Sha256};
            let held = self.mcp.lock().map_err(|_| EgressError::Configuration)?;
            match held.as_ref() {
                Some(owner) => owner
                    .claurst_servers(&format!("{:x}", Sha256::digest(contents.as_bytes())))
                    .map_err(|_| EgressError::Configuration)?,
                None => None,
            }
        };
        #[cfg(not(windows))]
        let clients = None;
        let settings = claurst_mcp_settings::settings(contents, clients)?;
        let directory = self.private_state.root().join("home/.claurst");
        std::fs::create_dir(&directory).map_err(|_| EgressError::Configuration)?;
        std::fs::write(
            directory.join("settings.json"),
            serde_json::to_vec(&settings).map_err(|_| EgressError::Configuration)?,
        )
        .map_err(|_| EgressError::Configuration)
    }
}

#[cfg(test)]
fn contained_settings(contents: &str) -> Result<serde_json::Value, EgressError> {
    claurst_mcp_settings::settings(contents, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contained_model_settings_bind_authenticated_adapter_without_secret() {
        let settings=contained_settings(r#"{"config":{"provider":"llama-cpp","model":"llama-cpp/qwen/local","mcp_servers":[],"api_key":"old-secret","provider_configs":{"llama-cpp":{"api_base":"http://other"}}},"providers":{"llamacpp":{"api_base":"http://other"}}}"#).unwrap();
        assert_eq!(settings["config"]["model"], "custom-openai/qwen/local");
        assert_eq!(settings["config"]["provider"], "custom-openai");
        assert_eq!(settings["config"]["api_key"], "{env:CUSTOM_OPENAI_API_KEY}");
        assert_eq!(
            settings["config"]["provider_configs"],
            serde_json::json!({})
        );
        let encoded = serde_json::to_string(&settings).unwrap();
        assert!(!encoded.contains("old-secret") && !encoded.contains("http://other"));
        assert_eq!(encoded.matches("__gent_claurst_proxy_v1__").count(), 1);
        for invalid in [
            "[]",
            "{}",
            r#"{"config":{"provider":"llama-cpp","model":"llama-cpp/x","mcp_servers":[{}]}}"#,
            r#"{"config":{"provider":"other","model":"llama-cpp/x","mcp_servers":[]}}"#,
            r#"{"config":{"provider":"llama-cpp","model":"remote/x","mcp_servers":[]}}"#,
            r#"{"config":{"provider":"llama-cpp","model":"llama-cpp/","mcp_servers":[]}}"#,
        ] {
            assert!(contained_settings(invalid).is_err());
        }
    }
}
