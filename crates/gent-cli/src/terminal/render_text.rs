use super::UiState;
use gent_types::{NormalizedTranscriptEvent, NormalizedTranscriptKind};

pub(super) fn conversation_title(events: &[NormalizedTranscriptEvent]) -> String {
    let text = events
        .iter()
        .find(|event| event.kind == NormalizedTranscriptKind::UserMessage)
        .map(|event| event.text.as_str())
        .unwrap_or("Untitled conversation");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let text = text
        .strip_prefix("please ")
        .or_else(|| text.strip_prefix("Please "))
        .unwrap_or(&text);
    clip(text, 58)
}

pub(crate) fn selected_title(state: &UiState) -> String {
    state
        .selected()
        .and_then(|item| title_for(state, &item.conversation_id))
        .unwrap_or_else(|| conversation_title(state.selected_transcript()))
}

pub(super) fn title_for(state: &UiState, conversation_id: &str) -> Option<String> {
    let title: Option<String> = state
        .metadata(conversation_id)
        .and_then(|metadata| metadata.title.clone())
        .filter(|title| !title.trim().is_empty());
    title
}

pub(super) fn clip(text: &str, limit: usize) -> String {
    let mut value = text.chars().take(limit).collect::<String>();
    if text.chars().count() > limit {
        value.push('…');
    }
    value
}

pub(super) fn plural(count: u32) -> &'static str {
    if count == 1 { "" } else { "s" }
}
use gent_types::{ConversationLiveStatus, ConversationStatus};

pub(super) fn status_activity(status: &ConversationStatus) -> &'static str {
    status
        .runs
        .iter()
        .filter_map(|run| run.live_status.as_ref())
        .map(|live| activity_text(&live.status))
        .find(|value| *value != "idle")
        .unwrap_or("idle")
}

fn activity_text(status: &ConversationLiveStatus) -> &'static str {
    if status.attention.is_required() {
        "attention needed"
    } else if status.error.has_error() {
        "error"
    } else if status.subagent_work.is_waiting() {
        "subagents working"
    } else if status.command_work.is_waiting() {
        "command running"
    } else if status.processing.is_processing() {
        "working"
    } else {
        "idle"
    }
}
