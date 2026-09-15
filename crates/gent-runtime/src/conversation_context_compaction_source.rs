use std::collections::{BTreeMap, VecDeque};

use gent_types::{
    ContextCompactionTrigger, ContextSourceItem, ContextSourceRole, ConversationContentEntry,
    NormalizedTranscriptEvent, NormalizedTranscriptKind,
};

use super::{ContextCompactionBudget, SOURCE_ITEM_OVERHEAD_BYTES};
use crate::conversation_context_window::weight;

pub(super) fn budget_target(
    candidates: &[ConversationContentEntry],
    events: &VecDeque<NormalizedTranscriptEvent>,
    budget: ContextCompactionBudget,
) -> Option<u64> {
    let turns = by_turn(events);
    let (mut bytes, mut items) = (0, 0);
    for (kept, entry) in candidates.iter().rev().enumerate() {
        let owned = turns
            .get(entry.turn_id.as_str())
            .map_or(&[][..], Vec::as_slice);
        bytes += weight(&entry.text) + owned.iter().map(|event| weight(&event.text)).sum::<usize>();
        items += owned.len();
        if kept >= budget.tail_entries
            || bytes > budget.tail_bytes
            || items > budget.tail_transcript_items
        {
            return Some(entry.ordinal);
        }
    }
    None
}

fn by_turn(
    events: &VecDeque<NormalizedTranscriptEvent>,
) -> BTreeMap<&str, Vec<&NormalizedTranscriptEvent>> {
    let mut turns = BTreeMap::<&str, Vec<_>>::new();
    for event in events {
        turns.entry(event.turn_id.as_str()).or_default().push(event);
    }
    turns
}

pub(super) fn source_items(
    candidates: &[ConversationContentEntry],
    events: &VecDeque<NormalizedTranscriptEvent>,
    target: u64,
    through: u64,
    trigger: ContextCompactionTrigger,
    budget: ContextCompactionBudget,
) -> (Vec<ContextSourceItem>, u32) {
    let turns = by_turn(events);
    let mut items = events
        .iter()
        .filter(|event| event.event_id.starts_with("import:"))
        .filter_map(|event| item(event, budget.item_bytes))
        .collect::<Vec<_>>();
    for entry in candidates.iter().filter(|entry| entry.ordinal <= target) {
        if trigger == ContextCompactionTrigger::Command && entry.ordinal == through {
            continue;
        }
        items.push(ContextSourceItem {
            role: ContextSourceRole::User,
            text: bounded(&entry.text, budget.item_bytes),
        });
        for event in turns.get(entry.turn_id.as_str()).into_iter().flatten() {
            items.extend(item(event, budget.item_bytes));
        }
    }
    let mut bytes = 0;
    let kept = items
        .iter()
        .rev()
        .take_while(|item| {
            bytes += item.text.len() + SOURCE_ITEM_OVERHEAD_BYTES;
            bytes <= budget.source_bytes
        })
        .count();
    let omitted = items.len() - kept;
    items.drain(..omitted);
    (items, u32::try_from(omitted).unwrap_or(u32::MAX))
}

fn item(event: &NormalizedTranscriptEvent, limit: usize) -> Option<ContextSourceItem> {
    let role = match event.kind {
        NormalizedTranscriptKind::UserMessage if event.event_id.starts_with("import:") => {
            ContextSourceRole::User
        }
        NormalizedTranscriptKind::AssistantMessage
            if event
                .event_id
                .starts_with(gent_types::INTERRUPTED_REPLY_EVENT_PREFIX) =>
        {
            ContextSourceRole::InterruptedAssistant
        }
        NormalizedTranscriptKind::AssistantMessage => ContextSourceRole::Assistant,
        NormalizedTranscriptKind::ToolActivity => ContextSourceRole::Tool,
        NormalizedTranscriptKind::Notice => ContextSourceRole::Notice,
        NormalizedTranscriptKind::Plan => ContextSourceRole::Plan,
        NormalizedTranscriptKind::UserMessage | NormalizedTranscriptKind::Thinking => return None,
    };
    Some(ContextSourceItem {
        role,
        text: bounded(&event.text, limit),
    })
}

fn bounded(text: &str, limit: usize) -> String {
    gent_types::bounded_text(text, limit).into_owned()
}
