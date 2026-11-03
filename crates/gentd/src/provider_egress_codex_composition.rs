use super::{
    Arc, AtomicBool, Coordinator, EgressError, Path, ProviderApi, ProviderEgressBroker,
    ProviderGatewayLease, ReviewedProviderEgress, RunFence, RunVersionLock, SandboxPrivateState,
    SqliteLedger,
};

impl ProviderGatewayLease {
    pub(crate) fn codex_managed(
        ledger: SqliteLedger,
        coordinator: Coordinator<SqliteLedger>,
        conversation_id: String,
        run_id: String,
        lock: RunVersionLock,
        probe_root: &Path,
        resources: &crate::windows_provider_resources::ManagedProviderResources,
    ) -> Result<Arc<Self>, EgressError> {
        let fence = RunFence::capture(ledger, coordinator, conversation_id, run_id, lock)?;
        if fence.lock.provider != "codex" || !fence.current() {
            return Err(EgressError::Configuration);
        }
        let credential = super::super::codex_credential::resolve(
            &fence.lock,
            Path::new(&fence.workspace.canonical_path),
            resources,
        )?;
        if !fence.current() {
            return Err(EgressError::Configuration);
        }
        let private_root = tempfile::Builder::new()
            .prefix("codex-state-")
            .tempdir_in(probe_root)
            .map_err(|_| EgressError::Configuration)?;
        let canonical =
            std::fs::canonicalize(private_root.path()).map_err(|_| EgressError::Configuration)?;
        let private_state = SandboxPrivateState::new(fence.run_id.clone(), canonical)
            .map_err(|_| EgressError::Configuration)?;
        for directory in private_state.directories() {
            std::fs::create_dir(&directory).map_err(|_| EgressError::Configuration)?;
        }
        let policy = ReviewedProviderEgress::new(
            ProviderApi::Codex,
            credential.upstream,
            credential.headers,
        )?;
        let fence = Arc::new(fence);
        let current = fence.clone();
        let broker =
            ProviderEgressBroker::start_checked(policy, Arc::new(move || current.current()))?;
        Ok(Arc::new(Self {
            fence,
            broker,
            private_state,
            _private_root: private_root,
            retained_resources: std::sync::Mutex::new(Vec::new()),
            revoked: AtomicBool::new(false),
            contained_model: None,
            mcp: std::sync::Mutex::default(),
        }))
    }
}
