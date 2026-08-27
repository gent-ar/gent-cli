use gent_ports::{AgentChatReadLedger, ConversationArtifactLedger, ConversationSummaryRunner};
use gent_types::{ConversationArtifact, ConversationArtifactKind, ReceiptId};

use crate::{AgentChatReadService, RuntimeError};

use super::{
    conversation_summary::{ConversationSummaryKind, scheduled_requests},
    conversation_summary_service::ConversationSummaryService,
};

#[derive(Debug)]
pub struct ConversationSummaryScheduler<L, R> {
    ledger: L,
    runner: R,
}

impl<L, R> ConversationSummaryScheduler<L, R>
where
    L: Clone + AgentChatReadLedger + ConversationArtifactLedger,
    R: Clone + ConversationSummaryRunner,
{
    #[must_use]
    pub fn new(ledger: L, runner: R) -> Self {
        Self { ledger, runner }
    }

    pub fn schedule(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<ConversationArtifact>, RuntimeError> {
        let reads = AgentChatReadService::new(self.ledger.clone());
        let detail = reads.detail(conversation_id)?;
        let events = transcript(&reads, conversation_id)?;
        let artifacts = self.ledger.list_conversation_artifacts(conversation_id)?;
        let requests = scheduled_requests(
            conversation_id,
            provider_name(detail.summary.selection.provider),
            &detail.summary.selection.model,
            &events,
            &artifacts,
        )?;
        let service = ConversationSummaryService::new(self.ledger.clone(), self.runner.clone());
        let mut generated = Vec::new();
        let mut failure = None;
        for request in requests {
            let supersedes =
                latest(&artifacts, request.kind).map(|value| value.artifact_id.clone());
            match service.generate(
                &request,
                format!("summary:{}", ReceiptId::new().0),
                supersedes,
            ) {
                Ok(artifact) => generated.push(artifact),
                Err(error) => {
                    if failure.is_none() {
                        failure = Some(error);
                    }
                }
            }
        }
        failure.map_or(Ok(generated), Err)
    }
}

fn transcript<L: AgentChatReadLedger>(
    reads: &AgentChatReadService<L>,
    conversation_id: &str,
) -> Result<Vec<gent_types::NormalizedTranscriptEvent>, RuntimeError> {
    let mut after = None;
    let mut events = Vec::new();
    loop {
        let page = reads.transcript(conversation_id, after, 100)?;
        events.extend(page.events);
        let Some(cursor) = page.next_after_cursor else {
            return Ok(events);
        };
        after = Some(cursor);
    }
}

fn provider_name(provider: gent_types::AgentChatProvider) -> &'static str {
    match provider {
        gent_types::AgentChatProvider::Claude => "claude",
        gent_types::AgentChatProvider::Codex => "codex",
        gent_types::AgentChatProvider::Claurst => "claurst",
    }
}

fn latest(
    artifacts: &[ConversationArtifact],
    kind: ConversationSummaryKind,
) -> Option<&ConversationArtifact> {
    let kind = match kind {
        ConversationSummaryKind::Title => ConversationArtifactKind::Title,
        ConversationSummaryKind::Recap => ConversationArtifactKind::Recap,
    };
    artifacts
        .iter()
        .rev()
        .find(|artifact| artifact.kind == kind)
}
