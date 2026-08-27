use gent_ports::{ConversationArtifactLedger, ConversationSummaryRunner};
use gent_types::{ConversationArtifact, ConversationArtifactStatus};

use super::conversation_summary::{ConversationSummaryRequest, complete};

#[derive(Debug)]
pub struct ConversationSummaryService<L, R> {
    ledger: L,
    runner: R,
}

impl<L, R> ConversationSummaryService<L, R>
where
    L: ConversationArtifactLedger,
    R: ConversationSummaryRunner,
{
    pub fn new(ledger: L, runner: R) -> Self {
        Self { ledger, runner }
    }

    pub fn generate(
        &self,
        request: &ConversationSummaryRequest,
        artifact_id: String,
        supersedes_artifact_id: Option<String>,
    ) -> Result<ConversationArtifact, crate::RuntimeError> {
        let result = self
            .runner
            .run_summary(&request.provider, &request.model_version, &request.prompt)
            .map_err(crate::RuntimeError::Port)
            .and_then(|response| {
                complete(
                    request,
                    artifact_id.clone(),
                    &response,
                    supersedes_artifact_id.clone(),
                )
                .map_err(crate::RuntimeError::ConversationSummary)
            });
        match result {
            Ok(response) => {
                self.ledger.create_conversation_artifact(&response)?;
                Ok(response)
            }
            Err(error) => {
                let artifact = failed(request, artifact_id, supersedes_artifact_id);
                self.ledger.create_conversation_artifact(&artifact)?;
                Err(error)
            }
        }
    }
}

fn failed(
    request: &ConversationSummaryRequest,
    artifact_id: String,
    supersedes_artifact_id: Option<String>,
) -> ConversationArtifact {
    ConversationArtifact {
        artifact_id,
        conversation_id: request.conversation_id.clone(),
        kind: request.kind.artifact_kind(),
        source_turn_ids: request.source_turn_ids.clone(),
        provider: request.provider.clone(),
        model_version: request.model_version.clone(),
        input_digest: request.input_digest.clone(),
        status: ConversationArtifactStatus::Failed,
        text: None,
        supersedes_artifact_id,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use gent_ports::{ConversationArtifactLedger, LedgerError, PortError};

    use super::{ConversationSummaryRequest, ConversationSummaryService};

    #[derive(Clone, Debug, Default)]
    struct Ledger(Arc<Mutex<Vec<gent_types::ConversationArtifact>>>);

    impl ConversationArtifactLedger for Ledger {
        fn create_conversation_artifact(
            &self,
            artifact: &gent_types::ConversationArtifact,
        ) -> Result<(), LedgerError> {
            self.0.lock().unwrap().push(artifact.clone());
            Ok(())
        }

        fn list_conversation_artifacts(
            &self,
            _: &str,
        ) -> Result<Vec<gent_types::ConversationArtifact>, LedgerError> {
            Ok(self.0.lock().unwrap().clone())
        }
    }

    #[derive(Debug)]
    struct Runner;

    impl gent_ports::ConversationSummaryRunner for Runner {
        fn run_summary(&self, _: &str, _: &str, _: &str) -> Result<String, PortError> {
            Ok("invalid".into())
        }
    }

    #[test]
    fn malformed_provider_output_persists_a_failed_artifact() {
        let ledger = Ledger::default();
        let service = ConversationSummaryService::new(ledger.clone(), Runner);
        assert!(
            service
                .generate(
                    &ConversationSummaryRequest {
                        conversation_id: "conversation".into(),
                        kind: super::super::conversation_summary::ConversationSummaryKind::Title,
                        source_turn_ids: vec!["turn".into()],
                        provider: "claude".into(),
                        model_version: "haiku".into(),
                        input_digest: "a".repeat(64),
                        prompt: "prompt".into(),
                    },
                    "artifact".into(),
                    None,
                )
                .is_err()
        );
        assert_eq!(
            ledger.0.lock().unwrap()[0].status,
            gent_types::ConversationArtifactStatus::Failed
        );
    }
}
