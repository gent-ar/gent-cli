use super::{ModelEndpoint, ModelError, ModelRequest, protocol};
impl ModelEndpoint {
    #[cfg(test)]
    pub(crate) fn test_root(&self) -> std::path::PathBuf {
        self.scope.root_path()
    }
    #[cfg(test)]
    pub(crate) fn test_force_helper_kill(&self) {
        use gent_drivers::interrupt::ProcessTreeSignal;
        assert!(
            self.scope
                .process
                .signal_tree(ProcessTreeSignal::Kill)
                .is_err()
        );
    }
    pub(crate) fn json(&self, body: Vec<u8>) -> Result<serde_json::Value, ModelError> {
        let mut response = Vec::new();
        self.respond(
            ModelRequest {
                method: "POST".into(),
                path: "/v1/chat/completions".into(),
                body,
            },
            &mut response,
        )
        .map_err(|error| error.error)?;
        protocol::json_response(&response)
    }
    pub(crate) fn matches_authority(&self, run: &str, sdk: &gent_types::RunVersionLock) -> bool {
        self.scope.run == run && self.sdk_lock.as_ref() == Some(sdk)
    }
    pub(crate) fn current(&self) -> bool {
        self.scope.current()
    }
    pub(crate) fn identity(&self) -> String {
        format!(
            "model-stdio-v1:{}:{}:{}:{}",
            self.scope.run,
            self.scope.request.lock.digest_sha256,
            self.scope.model_digest,
            self.scope.request.profile.digest_sha256()
        )
    }
    pub(crate) fn revoke(&self) -> Result<(), ModelError> {
        self.scope.retire()
    }
    pub(crate) fn settled(&self) -> bool {
        self.scope.settled()
    }
}
