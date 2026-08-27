use gent_types::{
    ConversationArtifact, ConversationArtifactKind, ConversationArtifactStatus,
    NormalizedTranscriptEvent, NormalizedTranscriptKind,
};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

const MAX_TRANSCRIPT_BYTES: usize = 24_000;
const TRANSCRIPT_HEAD_BYTES: usize = 4_000;
const OMITTED_TRANSCRIPT: &str =
    "\n\n[Earlier conversation content omitted for sidebar metadata]\n\n";
const MAX_OUTPUT_BYTES: usize = 16 * 1024;
const MAX_TITLE_BYTES: usize = 60;
const MAX_RECAP_BYTES: usize = 200;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConversationSummaryKind {
    Title,
    Recap,
}

impl ConversationSummaryKind {
    pub(crate) fn artifact_kind(self) -> ConversationArtifactKind {
        match self {
            Self::Title => ConversationArtifactKind::Title,
            Self::Recap => ConversationArtifactKind::Recap,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationSummaryRequest {
    pub conversation_id: String,
    pub kind: ConversationSummaryKind,
    pub source_turn_ids: Vec<String>,
    pub provider: String,
    pub model_version: String,
    pub input_digest: String,
    pub prompt: String,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationSummaryResult {
    pub title: Option<String>,
    pub recap: Option<String>,
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum ConversationSummaryError {
    #[error("summary input is empty")]
    EmptyInput,
    #[error("summary input exceeds its byte bound")]
    InputTooLarge,
    #[error("summary response is invalid")]
    InvalidResponse,
    #[error("summary response exceeds its byte bound")]
    OutputTooLarge,
    #[error("summary response has no usable content")]
    EmptyResponse,
}

#[must_use]
pub const fn summary_due(kind: ConversationSummaryKind, completed_turns: usize) -> bool {
    match kind {
        ConversationSummaryKind::Title => completed_turns >= 1,
        ConversationSummaryKind::Recap => {
            completed_turns >= 6 && (completed_turns - 6).is_multiple_of(6)
        }
    }
}

pub fn scheduled_requests(
    conversation_id: &str,
    provider: &str,
    model_version: &str,
    events: &[NormalizedTranscriptEvent],
    artifacts: &[ConversationArtifact],
) -> Result<Vec<ConversationSummaryRequest>, ConversationSummaryError> {
    let source_turn_ids = completed_turn_ids(events);
    let completed_turns = source_turn_ids.len();
    let mut requests = Vec::new();
    if summary_due(ConversationSummaryKind::Title, completed_turns)
        && completed_artifact(artifacts, ConversationArtifactKind::Title).is_none()
    {
        requests.push(request(
            conversation_id,
            ConversationSummaryKind::Title,
            source_turn_ids.clone(),
            provider,
            model_version,
            events,
        )?);
    }
    if summary_due(ConversationSummaryKind::Recap, completed_turns)
        && completed_artifact(artifacts, ConversationArtifactKind::Recap)
            .is_none_or(|artifact| artifact.source_turn_ids != source_turn_ids)
    {
        requests.push(request(
            conversation_id,
            ConversationSummaryKind::Recap,
            source_turn_ids,
            provider,
            model_version,
            events,
        )?);
    }
    Ok(requests)
}

pub fn request(
    conversation_id: &str,
    kind: ConversationSummaryKind,
    source_turn_ids: Vec<String>,
    provider: &str,
    model_version: &str,
    events: &[NormalizedTranscriptEvent],
) -> Result<ConversationSummaryRequest, ConversationSummaryError> {
    if conversation_id.trim().is_empty()
        || source_turn_ids.is_empty()
        || provider.trim().is_empty()
        || model_version.trim().is_empty()
        || events.is_empty()
    {
        return Err(ConversationSummaryError::EmptyInput);
    }
    let transcript = compact(
        events
            .iter()
            .filter(|event| {
                !event.is_partial
                    && matches!(
                        event.kind,
                        NormalizedTranscriptKind::UserMessage
                            | NormalizedTranscriptKind::AssistantMessage
                            | NormalizedTranscriptKind::ToolActivity
                            | NormalizedTranscriptKind::Notice
                    )
            })
            .map(|event| format!("{}: {}", label(event.kind), event.text.trim()))
            .filter(|line| !line.ends_with(": "))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    if transcript.is_empty() {
        return Err(ConversationSummaryError::EmptyInput);
    }
    let instruction = match kind {
        ConversationSummaryKind::Title => {
            "Return JSON with one short `title` string and an empty `recap` string."
        }
        ConversationSummaryKind::Recap => {
            "Return JSON with an empty `title` string and a concise `recap` string."
        }
    };
    let prompt = format!(
        "Summarize this Gent conversation. {instruction} Do not use markdown or commentary.\n\n{transcript}"
    );
    let input_digest = digest(&prompt);
    Ok(ConversationSummaryRequest {
        conversation_id: conversation_id.into(),
        kind,
        source_turn_ids,
        provider: provider.into(),
        model_version: model_version.into(),
        input_digest,
        prompt,
    })
}

pub fn complete(
    request: &ConversationSummaryRequest,
    artifact_id: String,
    response: &str,
    supersedes_artifact_id: Option<String>,
) -> Result<ConversationArtifact, ConversationSummaryError> {
    if response.len() > MAX_OUTPUT_BYTES {
        return Err(ConversationSummaryError::OutputTooLarge);
    }
    let parsed: ConversationSummaryResult = serde_json::from_str(json_object(response))
        .map_err(|_| ConversationSummaryError::InvalidResponse)?;
    let text = match request.kind {
        ConversationSummaryKind::Title => parsed.title,
        ConversationSummaryKind::Recap => parsed.recap,
    }
    .map(|value| value.split_whitespace().collect::<Vec<_>>().join(" "))
    .filter(|value| !value.is_empty())
    .ok_or(ConversationSummaryError::EmptyResponse)?;
    let max_bytes = match request.kind {
        ConversationSummaryKind::Title => MAX_TITLE_BYTES,
        ConversationSummaryKind::Recap => MAX_RECAP_BYTES,
    };
    let text = clip(text, max_bytes);
    Ok(ConversationArtifact {
        artifact_id,
        conversation_id: request.conversation_id.clone(),
        kind: request.kind.artifact_kind(),
        source_turn_ids: request.source_turn_ids.clone(),
        provider: request.provider.clone(),
        model_version: request.model_version.clone(),
        input_digest: request.input_digest.clone(),
        status: ConversationArtifactStatus::Completed,
        text: Some(text),
        supersedes_artifact_id,
    })
}

fn label(kind: NormalizedTranscriptKind) -> &'static str {
    match kind {
        NormalizedTranscriptKind::UserMessage => "user",
        NormalizedTranscriptKind::AssistantMessage => "assistant",
        NormalizedTranscriptKind::ToolActivity => "tool",
        NormalizedTranscriptKind::Notice => "notice",
        NormalizedTranscriptKind::Thinking => "thinking",
    }
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn completed_turn_ids(events: &[NormalizedTranscriptEvent]) -> Vec<String> {
    let mut seen = HashSet::new();
    events
        .iter()
        .filter(|event| {
            event.kind == NormalizedTranscriptKind::AssistantMessage
                && !event.is_partial
                && seen.insert(event.turn_id.clone())
        })
        .map(|event| event.turn_id.clone())
        .collect()
}

fn completed_artifact(
    artifacts: &[ConversationArtifact],
    kind: ConversationArtifactKind,
) -> Option<&ConversationArtifact> {
    artifacts.iter().rev().find(|artifact| {
        artifact.kind == kind && artifact.status == ConversationArtifactStatus::Completed
    })
}

fn compact(value: String) -> String {
    if value.len() <= MAX_TRANSCRIPT_BYTES {
        return value;
    }
    let tail = MAX_TRANSCRIPT_BYTES - TRANSCRIPT_HEAD_BYTES - OMITTED_TRANSCRIPT.len();
    let head_end = boundary(&value, TRANSCRIPT_HEAD_BYTES);
    let tail_start = boundary(&value, value.len() - tail);
    format!(
        "{}{}{}",
        &value[..head_end],
        OMITTED_TRANSCRIPT,
        &value[tail_start..]
    )
}

fn json_object(value: &str) -> &str {
    let start = value.find('{').unwrap_or(value.len());
    let end = value.rfind('}').map_or(start, |index| index + 1);
    &value[start..end]
}

fn clip(mut value: String, limit: usize) -> String {
    if value.len() <= limit {
        return value;
    }
    value.truncate(boundary(&value, limit));
    if let Some(index) = value.rfind(char::is_whitespace) {
        return value[..index].trim_end().into();
    }
    value
}

fn boundary(value: &str, index: usize) -> usize {
    let mut index = index.min(value.len());
    while !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[cfg(test)]
#[path = "conversation_summary_tests.rs"]
mod tests;
