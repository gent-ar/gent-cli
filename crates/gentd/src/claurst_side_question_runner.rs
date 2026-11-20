use std::{sync::Arc, thread};

use async_trait::async_trait;
use gent_drivers::conversation_context_summary::SummaryRequest;
use gent_ports::{ConversationSummaryRunner, PortError};
use gent_types::ContextCompactionFailure;

use crate::claurst_runtime_factory::ContextSummarizer;
use crate::standalone_claurst_runtime_factory::StandaloneClaurstBridge;

const INSTRUCTIONS: &str = "Answer the user's side question directly and concisely in plain text.";

#[async_trait]
pub(crate) trait ClaurstSideQuestionModels: Send + Sync + std::fmt::Debug {
    async fn answer_model(
        &self,
        conversation_id: &str,
        model_id: &str,
    ) -> Result<Arc<dyn ContextSummarizer>, PortError>;
}

#[async_trait]
impl ClaurstSideQuestionModels for StandaloneClaurstBridge {
    async fn answer_model(
        &self,
        conversation_id: &str,
        model_id: &str,
    ) -> Result<Arc<dyn ContextSummarizer>, PortError> {
        let model = self
            .runtime()
            .side_question_model(conversation_id, model_id)
            .await?;
        Ok(model)
    }
}

#[derive(Debug)]
pub(crate) struct ClaurstSideQuestionRunner {
    models: Arc<dyn ClaurstSideQuestionModels>,
    conversation_id: String,
}

impl ClaurstSideQuestionRunner {
    pub(crate) fn new(models: Arc<dyn ClaurstSideQuestionModels>, conversation_id: &str) -> Self {
        Self {
            models,
            conversation_id: conversation_id.to_owned(),
        }
    }
}

impl ConversationSummaryRunner for ClaurstSideQuestionRunner {
    fn run_summary(
        &self,
        provider: &str,
        model_version: &str,
        prompt: &str,
    ) -> Result<String, PortError> {
        if provider != "claurst" || model_version.trim().is_empty() || prompt.trim().is_empty() {
            return Err(PortError::Unavailable(
                "a Gent local side question needs the selected local model".into(),
            ));
        }
        let models = Arc::clone(&self.models);
        let conversation_id = self.conversation_id.clone();
        let model = model_version.to_owned();
        let content = prompt.to_owned();
        thread::Builder::new()
            .name("gent-claurst-side-question".into())
            .spawn(move || {
                tokio::runtime::Runtime::new()
                    .map_err(|_| PortError::Unavailable("Claurst side question runtime".into()))?
                    .block_on(async move {
                        models
                            .answer_model(&conversation_id, &model)
                            .await?
                            .summarize(SummaryRequest {
                                instructions: INSTRUCTIONS,
                                content,
                            })
                            .await
                            .map(|answer| answer.text)
                            .map_err(failure)
                    })
            })
            .map_err(|_| PortError::Unavailable("Claurst side question worker".into()))?
            .join()
            .map_err(|_| PortError::Unavailable("Claurst side question worker".into()))?
    }
}

fn failure(failure: ContextCompactionFailure) -> PortError {
    match failure {
        ContextCompactionFailure::RuntimeUnavailable => {
            PortError::Unavailable("the local model did not answer".into())
        }
        ContextCompactionFailure::OutputLimit => {
            PortError::Provider("the local model's answer exceeded its output bound".into())
        }
        ContextCompactionFailure::EmptySummary => {
            PortError::Provider("the local model returned no answer".into())
        }
    }
}
