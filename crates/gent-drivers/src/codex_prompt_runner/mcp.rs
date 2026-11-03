use gent_ports::PublicProviderRunError;
use sha2::{Digest, Sha256};

use super::CodexPromptRunner;

impl<L, P> CodexPromptRunner<L, P> {
    pub fn current_mcp_servers(
        &self,
    ) -> Result<Option<(serde_json::Value, String)>, PublicProviderRunError> {
        let Some(path) = &self.mcp_config else {
            return Ok(self
                .mcp_servers
                .clone()
                .map(|servers| (servers, String::new())));
        };
        let bytes = std::fs::read(path).map_err(|_| {
            PublicProviderRunError::Failed("Codex MCP config is unavailable".into())
        })?;
        if bytes.len() > 1024 * 1024 {
            return Err(PublicProviderRunError::Failed(
                "Codex MCP config is invalid".into(),
            ));
        }
        let servers = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|value| value.get("mcpServers").cloned())
            .filter(serde_json::Value::is_object)
            .ok_or_else(|| PublicProviderRunError::Failed("Codex MCP config is invalid".into()))?;
        Ok(Some((servers, hex::encode(Sha256::digest(bytes)))))
    }
}

impl<L, P> CodexPromptRunner<L, P>
where
    L: crate::supervisor::ProcessLauncher<Process = P>,
    P: crate::supervisor::ProviderProcess,
{
    /// Inspects the actual live start map while the provider process owner remains locked.
    pub fn with_owned_mcp<R>(
        &self,
        run: &str,
        conversation: &str,
        inspect: impl FnOnce(
            Option<&serde_json::Value>,
            Option<&std::path::Path>,
            Option<&crate::ProcessSandboxEvidence>,
        ) -> R,
    ) -> R {
        let runner = super::lock(&self.runner);
        let selected = super::lock(&self.launched_mcp);
        let actual = runner
            .is_live(run)
            .then(|| selected.get(run))
            .flatten()
            .filter(|(bound, _)| bound == conversation)
            .map(|(_, servers)| servers);
        inspect(
            actual,
            self.mcp_config.as_deref().and_then(std::path::Path::parent),
            runner.sandbox_evidence(run),
        )
    }
}

use super::{ProcessLauncher, ProviderProcess};
use gent_ports::PublicProviderRunner;
use gent_types::RunVersionLock;
impl<L, P> PublicProviderRunner for CodexPromptRunner<L, P>
where
    L: ProcessLauncher<Process = P> + Send + Sync,
    P: ProviderProcess,
{
    fn start(
        &self,
        run_id: &str,
        lock_value: &RunVersionLock,
    ) -> Result<(), PublicProviderRunError> {
        self.launch(run_id, lock_value, None)
    }

    fn resume(
        &self,
        run_id: &str,
        lock_value: &RunVersionLock,
        session_id: &str,
    ) -> Result<(), PublicProviderRunError> {
        if session_id.trim().is_empty() {
            return Err(PublicProviderRunError::Failed(
                "Codex resume thread is invalid".into(),
            ));
        }
        self.launch(run_id, lock_value, Some(session_id.into()))
    }

    fn interrupt(&self, run_id: &str) -> Result<(), PublicProviderRunError> {
        self.signal_process(run_id, crate::interrupt::ProcessTreeSignal::Interrupt)
    }
}
impl<L, P> CodexPromptRunner<L, P>
where
    L: ProcessLauncher<Process = P>,
    P: ProviderProcess,
{
    /// Daemon binds the authoritative conversation before the existing prepare/start sequence.
    pub fn prepare_for_conversation(
        &self,
        run: String,
        conversation: String,
        prompt: super::CodexPromptStart,
    ) -> Result<(), PublicProviderRunError> {
        if conversation.trim().is_empty() {
            return Err(PublicProviderRunError::Failed(
                "MCP conversation is missing".into(),
            ));
        }
        self.prepare(run.clone(), prompt)?;
        super::lock(&self.pending_mcp_conversations).insert(run, conversation);
        Ok(())
    }
}
