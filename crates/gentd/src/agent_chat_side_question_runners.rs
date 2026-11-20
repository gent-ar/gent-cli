use std::{path::PathBuf, sync::Arc};

use gent_ports::{ConversationSummaryRunner, PortError};
use gent_types::AgentChatProvider;

use crate::{
    claude_summary_runner::ClaudeSummaryRunner,
    claurst_side_question_runner::{ClaurstSideQuestionModels, ClaurstSideQuestionRunner},
    codex_summary_runner::CodexSummaryRunner,
    provider_executables::ProviderExecutables,
};

const STREAM_CAPTURE_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct AgentChatSideQuestionRunnerSources {
    pub(crate) data_dir: PathBuf,
    pub(crate) executables: ProviderExecutables,
    pub(crate) claurst_models: Option<Arc<dyn ClaurstSideQuestionModels>>,
}

impl AgentChatSideQuestionRunnerSources {
    pub(crate) fn resolve(
        &self,
        provider: AgentChatProvider,
        workspace_path: Option<&str>,
        conversation_id: &str,
    ) -> Result<Box<dyn ConversationSummaryRunner>, PortError> {
        match provider {
            AgentChatProvider::Claude => self.claude_runner(),
            AgentChatProvider::Codex => self.codex_runner(workspace_path),
            AgentChatProvider::Claurst => self
                .claurst_models
                .clone()
                .map(|models| {
                    Box::new(ClaurstSideQuestionRunner::new(models, conversation_id))
                        as Box<dyn ConversationSummaryRunner>
                })
                .ok_or_else(|| PortError::Unavailable("Claurst is not attached".into())),
        }
    }

    fn claude_runner(&self) -> Result<Box<dyn ConversationSummaryRunner>, PortError> {
        let resolver = self
            .executables
            .locks(AgentChatProvider::Claude)
            .map_err(|error| PortError::Unavailable(error.to_string()))?;
        Ok(Box::new(ClaudeSummaryRunner::new(Arc::new(resolver))))
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
            Arc::new(resolver),
            workspace_root,
        )))
    }
}

#[cfg(test)]
#[path = "agent_chat_side_question_runners_tests.rs"]
mod tests;
