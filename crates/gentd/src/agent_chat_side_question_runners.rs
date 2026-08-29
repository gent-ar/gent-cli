//! Per-provider side-question runner resolution, kept separate from live provider hosts.
//!
//! Claude and Codex are resolved fresh, per call, from an installed executable: the same
//! stateless resolution `compose_standalone_claude`/`compose_standalone_codex` already use for
//! their own summary runners. This means side questions never depend on whether that
//! provider's lazy lifecycle host has already been started for a real turn. Claurst has no such
//! stateless resolution — its bridge depends on an already-running local runtime — so it is
//! threaded through directly instead.

use std::path::PathBuf;

use gent_drivers::{PublicProvider, process::SystemLauncher};
use gent_ports::{ConversationSummaryRunner, PortError, PublicProviderResolver};
use gent_types::AgentChatProvider;

use crate::{
    claude_summary_runner::ClaudeSummaryRunner, codex_summary_runner::CodexSummaryRunner,
    local_provider_locks::LocalProviderLocks,
    standalone_claurst_runtime_factory::StandaloneClaurstBridge,
    standalone_provider_setup::installed_provider_executable,
};

const STREAM_CAPTURE_BYTES: usize = 64 * 1024;

/// Everything needed to resolve a side-question runner for any provider a conversation might
/// currently be on, without depending on that provider's live lazy-started lifecycle host.
#[derive(Clone, Debug)]
pub(crate) struct AgentChatSideQuestionRunnerSources {
    pub(crate) data_dir: PathBuf,
    pub(crate) claude_executable: Option<PathBuf>,
    pub(crate) codex_executable: Option<PathBuf>,
    pub(crate) claurst_bridge: Option<StandaloneClaurstBridge>,
}

impl AgentChatSideQuestionRunnerSources {
    /// Resolves the runner for one conversation's current provider and workspace path.
    ///
    /// # Errors
    /// Returns an error when the provider is not installed, its executable no longer resolves,
    /// or (for Claurst) no local runtime is attached.
    pub(crate) fn resolve(
        &self,
        provider: AgentChatProvider,
        workspace_path: Option<&str>,
    ) -> Result<Box<dyn ConversationSummaryRunner>, PortError> {
        match provider {
            AgentChatProvider::Claude => self.claude_runner(),
            AgentChatProvider::Codex => self.codex_runner(workspace_path),
            AgentChatProvider::Claurst => self
                .claurst_bridge
                .clone()
                .map(|bridge| Box::new(bridge) as Box<dyn ConversationSummaryRunner>)
                .ok_or_else(|| PortError::Unavailable("Claurst is not attached".into())),
        }
    }

    fn claude_runner(&self) -> Result<Box<dyn ConversationSummaryRunner>, PortError> {
        let executable =
            self.resolved_executable(AgentChatProvider::Claude, self.claude_executable.clone())?;
        let lock = LocalProviderLocks::capture([(PublicProvider::Claude, executable)])
            .map_err(|error| PortError::Unavailable(error.to_string()))?
            .resolve("claude")
            .map_err(|error| PortError::Unavailable(error.to_string()))?;
        Ok(Box::new(ClaudeSummaryRunner::new(lock)?))
    }

    fn codex_runner(
        &self,
        workspace_path: Option<&str>,
    ) -> Result<Box<dyn ConversationSummaryRunner>, PortError> {
        let executable =
            self.resolved_executable(AgentChatProvider::Codex, self.codex_executable.clone())?;
        let lock = LocalProviderLocks::capture([(PublicProvider::Codex, executable)])
            .map_err(|error| PortError::Unavailable(error.to_string()))?
            .resolve("codex")
            .map_err(|error| PortError::Unavailable(error.to_string()))?;
        let workspace_root = workspace_path.map_or_else(|| self.data_dir.clone(), PathBuf::from);
        Ok(Box::new(CodexSummaryRunner::new(
            SystemLauncher::new(STREAM_CAPTURE_BYTES),
            lock,
            workspace_root,
        )))
    }

    fn resolved_executable(
        &self,
        provider: AgentChatProvider,
        explicit: Option<PathBuf>,
    ) -> Result<PathBuf, PortError> {
        explicit
            .or_else(|| installed_provider_executable(&self.data_dir, provider))
            .ok_or_else(|| PortError::Unavailable(format!("{provider:?} is not installed")))
    }
}
