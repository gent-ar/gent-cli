use gent_ports::{ContextCompactionLedger, ConversationContentReader, TranscriptLedger};
use gent_types::{
    AgentChatRunContext, ContextCompactionFact, ContextCompactionPlan, ContextCompactionTrigger,
    ContextCoverageDigest, ContextPolicy, ConversationContentEntry, ConversationContextSummary,
    FrozenConversationContext,
};

use super::{ConversationContextArtifactService, ConversationContextRequest, MAX_ENTRIES};
use crate::RuntimeError;

const COMPACTION_READ_LIMIT: u16 = 32;
const FAILURE_BACKOFF_ENTRIES: u64 = 8;
const SOURCE_ITEM_OVERHEAD_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContextCompactionBudget {
    pub tail_bytes: usize,
    pub tail_entries: usize,
    pub tail_transcript_items: usize,
    pub source_bytes: usize,
    pub item_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContextCompactionDecision {
    Plan(Box<ContextCompactionPlan>),
    NothingToCover,
    BackingOff,
}

struct RunView {
    request: ConversationContextRequest,
    entries: Vec<ConversationContentEntry>,
    imports: bool,
    prefix: Vec<(u64, String)>,
}

struct Covered {
    through: u64,
    imports: bool,
    summary: String,
}

impl<L> ConversationContextArtifactService<L>
where
    L: ConversationContentReader + TranscriptLedger + ContextCompactionLedger,
{
    pub fn project_run_summarized(
        &self,
        run: &AgentChatRunContext,
        message_id: &str,
    ) -> Result<Option<FrozenConversationContext>, RuntimeError> {
        let ordinal = self.message_ordinal(&run.conversation_id, message_id)?;
        let view = self.view(run, ordinal.saturating_sub(1))?;
        let facts = self
            .reader
            .context_compactions(&run.conversation_id.0, COMPACTION_READ_LIMIT)?;
        let Some(covered) = latest_covered(&view, &facts) else {
            return Ok(None);
        };
        let tail = view
            .entries
            .iter()
            .filter(|entry| entry.ordinal > covered.through)
            .cloned()
            .collect::<Vec<_>>();
        let omitted = tail.len() > MAX_ENTRIES;
        let tail = tail[tail.len().saturating_sub(MAX_ENTRIES)..].to_vec();
        self.artifact(
            &view.request,
            tail,
            view.imports && !covered.imports,
            Some(ConversationContextSummary {
                covers_through_ordinal: covered.through,
                imports_covered: covered.imports,
                text: covered.summary,
            }),
            omitted,
        )
        .map(Some)
    }

    pub fn plan_run_compaction(
        &self,
        run: &AgentChatRunContext,
        (message_id, turn_id): (&str, &str),
        trigger: ContextCompactionTrigger,
        budget: ContextCompactionBudget,
    ) -> Result<ContextCompactionDecision, RuntimeError> {
        let ordinal = self.message_ordinal(&run.conversation_id, message_id)?;
        let through = match trigger {
            ContextCompactionTrigger::Command => ordinal,
            ContextCompactionTrigger::Budget => ordinal.saturating_sub(1),
        };
        let view = self.view(run, through)?;
        let facts = self
            .reader
            .context_compactions(&run.conversation_id.0, COMPACTION_READ_LIMIT)?;
        if trigger == ContextCompactionTrigger::Budget && backing_off(&facts, through) {
            return Ok(ContextCompactionDecision::BackingOff);
        }
        let covered = latest_covered(&view, &facts);
        let after = covered.as_ref().map_or(0, |covered| covered.through);
        let candidates = view
            .entries
            .iter()
            .filter(|entry| entry.ordinal > after)
            .cloned()
            .collect::<Vec<_>>();
        let (events, _) = self.transcript(
            &view.request,
            &candidates,
            view.imports && covered.is_none(),
            usize::MAX,
        )?;
        let target = match trigger {
            ContextCompactionTrigger::Command => Some(through),
            ContextCompactionTrigger::Budget => budget_target(&candidates, &events, budget),
        };
        let Some(target) = target.filter(|target| *target > after) else {
            return Ok(ContextCompactionDecision::NothingToCover);
        };
        let (items, omitted) = source_items(&candidates, &events, target, through, trigger, budget);
        Ok(ContextCompactionDecision::Plan(Box::new(
            ContextCompactionPlan {
                conversation_id: run.conversation_id.0.clone(),
                run_id: run.run_id.0.clone(),
                turn_id: turn_id.to_owned(),
                trigger,
                covers_through_ordinal: target,
                covered_digest_sha256: prefix_digest(&view.prefix, target),
                imports_covered: view.imports,
                previous_summary: covered.map(|covered| covered.summary),
                items,
                omitted_source_items: omitted,
            },
        )))
    }

    fn view(&self, run: &AgentChatRunContext, through: u64) -> Result<RunView, RuntimeError> {
        let request = ConversationContextRequest {
            conversation_id: run.conversation_id.clone(),
            context_policy: ContextPolicy::Preserve,
            context_through_ordinal: through,
        };
        let preserve = run.context_policy == ContextPolicy::Preserve;
        let inherited = if preserve {
            run.context_through_ordinal
        } else {
            0
        };
        let mut entries = if through == 0 {
            Vec::new()
        } else {
            self.entries(&request, usize::MAX)?.0
        };
        entries.retain(|entry| entry.ordinal <= inherited || entry.run_id == run.run_id.0);
        let mut digest = ContextCoverageDigest::new(preserve);
        let mut prefix = vec![(0, digest.current())];
        for entry in &entries {
            digest.update(entry.ordinal, &entry.text_digest_sha256);
            prefix.push((entry.ordinal, digest.current()));
        }
        Ok(RunView {
            request,
            entries,
            imports: preserve,
            prefix,
        })
    }
}

fn latest_covered(view: &RunView, facts: &[ContextCompactionFact]) -> Option<Covered> {
    facts.iter().find_map(|fact| match fact {
        ContextCompactionFact::Compacted {
            covers_through_ordinal,
            covered_digest_sha256,
            imports_covered,
            summary,
            ..
        } if *covers_through_ordinal <= view.request.context_through_ordinal
            && prefix_digest(&view.prefix, *covers_through_ordinal) == *covered_digest_sha256 =>
        {
            Some(Covered {
                through: *covers_through_ordinal,
                imports: *imports_covered,
                summary: summary.clone(),
            })
        }
        _ => None,
    })
}

fn backing_off(facts: &[ContextCompactionFact], through: u64) -> bool {
    facts
        .iter()
        .find(|fact| {
            matches!(
                fact,
                ContextCompactionFact::Compacted { .. }
                    | ContextCompactionFact::Failed {
                        trigger: ContextCompactionTrigger::Budget,
                        ..
                    }
            )
        })
        .is_some_and(|fact| match fact {
            ContextCompactionFact::Failed {
                attempted_through_ordinal,
                ..
            } => attempted_through_ordinal.saturating_add(FAILURE_BACKOFF_ENTRIES) > through,
            ContextCompactionFact::Compacted { .. } => false,
        })
}

fn prefix_digest(prefix: &[(u64, String)], through: u64) -> String {
    let index = prefix.partition_point(|(ordinal, _)| *ordinal <= through);
    prefix[index.saturating_sub(1)].1.clone()
}

#[path = "conversation_context_compaction_source.rs"]
mod source;
use source::{budget_target, source_items};
