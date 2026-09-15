use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    PROVIDER_CONTEXT_COMPACTED_NOTICE, PROVIDER_CONTEXT_COMPACTION_FAILED_NOTICE,
    conversation_context::valid_sha256,
};

pub const CONTEXT_COMPACTION_EVENT_KIND: &str = "agentChatContextCompaction";
pub const MAX_CONTEXT_SUMMARY_BYTES: usize = 16 * 1024;
pub const CONTEXT_COMPACTION_FALLBACK_NOTICE: &str = "Gent could not compact this conversation's context, so its earliest history was left out of this prompt.";
pub const CONTEXT_COMPACTION_PARTIAL_NOTICE: &str = "Context compacted; the oldest history was too long to summarize and was left out of the summary.";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ContextCompactionTrigger {
    Command,
    Budget,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ContextCompactionFailure {
    RuntimeUnavailable,
    OutputLimit,
    EmptySummary,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ContextCompactionFact {
    Compacted {
        conversation_id: String,
        run_id: String,
        turn_id: String,
        trigger: ContextCompactionTrigger,
        covers_through_ordinal: u64,
        covered_digest_sha256: String,
        imports_covered: bool,
        summary: String,
        token_estimate: u32,
        omitted_source_items: u32,
    },
    Failed {
        conversation_id: String,
        run_id: String,
        turn_id: String,
        trigger: ContextCompactionTrigger,
        attempted_through_ordinal: u64,
        failure: ContextCompactionFailure,
    },
}

impl ContextCompactionFact {
    #[must_use]
    pub fn conversation_id(&self) -> &str {
        match self {
            Self::Compacted {
                conversation_id, ..
            }
            | Self::Failed {
                conversation_id, ..
            } => conversation_id,
        }
    }

    #[must_use]
    pub fn run_id(&self) -> &str {
        match self {
            Self::Compacted { run_id, .. } | Self::Failed { run_id, .. } => run_id,
        }
    }

    #[must_use]
    pub fn turn_id(&self) -> &str {
        match self {
            Self::Compacted { turn_id, .. } | Self::Failed { turn_id, .. } => turn_id,
        }
    }

    #[must_use]
    pub fn event_id(&self) -> String {
        format!("context-compaction:{}", self.turn_id())
    }

    #[must_use]
    pub fn notice_event_id(&self) -> String {
        format!("context-compaction-notice:{}", self.turn_id())
    }

    #[must_use]
    pub const fn notice(&self) -> &'static str {
        match self {
            Self::Compacted {
                omitted_source_items: 0,
                ..
            } => PROVIDER_CONTEXT_COMPACTED_NOTICE,
            Self::Compacted { .. } => CONTEXT_COMPACTION_PARTIAL_NOTICE,
            Self::Failed {
                trigger: ContextCompactionTrigger::Command,
                ..
            } => PROVIDER_CONTEXT_COMPACTION_FAILED_NOTICE,
            Self::Failed { .. } => CONTEXT_COMPACTION_FALLBACK_NOTICE,
        }
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        let identities = [self.conversation_id(), self.run_id(), self.turn_id()]
            .iter()
            .all(|value| !value.trim().is_empty() && value.len() <= 256);
        identities
            && match self {
                Self::Compacted {
                    covers_through_ordinal,
                    covered_digest_sha256,
                    summary,
                    ..
                } => {
                    *covers_through_ordinal > 0
                        && valid_sha256(covered_digest_sha256)
                        && !summary.trim().is_empty()
                        && summary.len() <= MAX_CONTEXT_SUMMARY_BYTES
                        && !summary.contains('\0')
                }
                Self::Failed {
                    attempted_through_ordinal,
                    ..
                } => *attempted_through_ordinal > 0,
            }
    }
}

#[derive(Clone, Debug)]
pub struct ContextCoverageDigest(Sha256);

impl ContextCoverageDigest {
    #[must_use]
    pub fn new(imports_covered: bool) -> Self {
        Self(
            Sha256::new()
                .chain_update(b"gent-context-compaction-v1\0")
                .chain_update([u8::from(imports_covered)]),
        )
    }

    pub fn update(&mut self, ordinal: u64, text_digest_sha256: &str) {
        self.0.update(ordinal.to_be_bytes());
        self.0.update(text_digest_sha256.as_bytes());
        self.0.update([0]);
    }

    #[must_use]
    pub fn current(&self) -> String {
        format!("{:x}", self.0.clone().finalize())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ContextSourceRole {
    User,
    Assistant,
    InterruptedAssistant,
    Tool,
    Notice,
    Plan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextSourceItem {
    pub role: ContextSourceRole,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextCompactionPlan {
    pub conversation_id: String,
    pub run_id: String,
    pub turn_id: String,
    pub trigger: ContextCompactionTrigger,
    pub covers_through_ordinal: u64,
    pub covered_digest_sha256: String,
    pub imports_covered: bool,
    pub previous_summary: Option<String>,
    pub items: Vec<ContextSourceItem>,
    pub omitted_source_items: u32,
}

impl ContextCompactionPlan {
    #[must_use]
    pub fn compacted(&self, summary: String, token_estimate: u32) -> ContextCompactionFact {
        ContextCompactionFact::Compacted {
            conversation_id: self.conversation_id.clone(),
            run_id: self.run_id.clone(),
            turn_id: self.turn_id.clone(),
            trigger: self.trigger,
            covers_through_ordinal: self.covers_through_ordinal,
            covered_digest_sha256: self.covered_digest_sha256.clone(),
            imports_covered: self.imports_covered,
            summary,
            token_estimate,
            omitted_source_items: self.omitted_source_items,
        }
    }

    #[must_use]
    pub fn failed(&self, failure: ContextCompactionFailure) -> ContextCompactionFact {
        ContextCompactionFact::Failed {
            conversation_id: self.conversation_id.clone(),
            run_id: self.run_id.clone(),
            turn_id: self.turn_id.clone(),
            trigger: self.trigger,
            attempted_through_ordinal: self.covers_through_ordinal,
            failure,
        }
    }
}

#[cfg(test)]
#[path = "conversation_context_compaction_tests.rs"]
mod tests;
