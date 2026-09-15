//! Per-provider side-question runner resolution, kept separate from live provider hosts.
//!
//! Claude and Codex are resolved fresh, per call, from an installed executable: the same
//! stateless resolution `compose_standalone_claude`/`compose_standalone_codex` already use for
//! their own summary runners. This means side questions never depend on whether that
//! provider's lazy lifecycle host has already been started for a real turn. Claurst has no such
//! stateless resolution — its bridge depends on an already-running local runtime — so it is
//! threaded through directly instead.

use std::path::PathBuf;

use gent_ports::{ConversationSummaryRunner, PortError};
use gent_types::AgentChatProvider;

use crate::{
    claude_summary_runner::ClaudeSummaryRunner, codex_summary_runner::CodexSummaryRunner,
    provider_executables::ProviderExecutables,
    standalone_claurst_runtime_factory::StandaloneClaurstBridge,
};

const STREAM_CAPTURE_BYTES: usize = 64 * 1024;

/// Everything needed to resolve a side-question runner for any provider a conversation might
/// currently be on, without depending on that provider's live lazy-started lifecycle host.
#[derive(Clone, Debug)]
pub(crate) struct AgentChatSideQuestionRunnerSources {
    pub(crate) data_dir: PathBuf,
    pub(crate) executables: ProviderExecutables,
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
        let resolver = self
            .executables
            .locks(AgentChatProvider::Claude)
            .map_err(|error| PortError::Unavailable(error.to_string()))?;
        Ok(Box::new(ClaudeSummaryRunner::new(std::sync::Arc::new(
            resolver,
        ))))
    }

    fn codex_runner(
        &self,
        workspace_path: Option<&str>,
    ) -> Result<Box<dyn ConversationSummaryRunner>, PortError> {
        let resolver = self
            .executables
            .locks(AgentChatProvider::Codex)
            .map_err(|error| PortError::Unavailable(error.to_string()))?;
        let workspace_root = workspace_path.map_or_else(|| self.data_dir.clone(), PathBuf::from);
        Ok(Box::new(CodexSummaryRunner::new(
            crate::node_runtime_lock::standalone_provider_launcher(STREAM_CAPTURE_BYTES),
            std::sync::Arc::new(resolver),
            workspace_root,
        )))
    }
}

#[cfg(test)]
#[path = "agent_chat_side_question_runners_tests.rs"]
mod tests;
