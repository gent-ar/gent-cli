use std::collections::{BTreeMap, VecDeque};

use gent_types::{
    ConversationContentEntry, INTERRUPTED_REPLY_EVENT_PREFIX, NormalizedTranscriptEvent,
    NormalizedTranscriptKind,
};

const ITEM_OVERHEAD_BYTES: usize = 256;

pub(crate) fn keep_newest<T>(retained: &mut VecDeque<T>, limit: usize) -> bool {
    let dropped = retained.len() > limit;
    while retained.len() > limit {
        retained.pop_front();
    }
    dropped
}

pub(crate) fn drop_oldest_until_within(
    entries: &mut VecDeque<ConversationContentEntry>,
    events: &mut VecDeque<NormalizedTranscriptEvent>,
    limit: usize,
) -> bool {
    let mut size = entries
        .iter()
        .map(|entry| weight(&entry.text))
        .chain(events.iter().map(|event| weight(&event.text)))
        .sum::<usize>();
    let dropped = size > limit;
    while size > limit {
        let imported = events
            .front()
            .is_some_and(|event| event.event_id.starts_with("import:"));
        let removed = if imported || entries.len() <= 1 {
            events.pop_front().map(|event| weight(&event.text))
        } else {
            drop_oldest_turn(entries, events)
        };
        let Some(removed) = removed else {
            return dropped;
        };
        size -= removed;
    }
    dropped
}

fn drop_oldest_turn(
    entries: &mut VecDeque<ConversationContentEntry>,
    events: &mut VecDeque<NormalizedTranscriptEvent>,
) -> Option<usize> {
    let entry = entries.pop_front()?;
    let mut removed = weight(&entry.text);
    events.retain(|event| {
        let owned = event.turn_id == entry.turn_id;
        if owned {
            removed += weight(&event.text);
        }
        !owned
    });
    Some(removed)
}

pub(crate) const fn weight(text: &str) -> usize {
    text.len() + ITEM_OVERHEAD_BYTES
}

#[derive(Default)]
pub(crate) struct UnfinishedReplies(BTreeMap<String, NormalizedTranscriptEvent>);

impl UnfinishedReplies {
    pub(crate) fn observe(&mut self, event: &NormalizedTranscriptEvent) {
        if event.kind != NormalizedTranscriptKind::AssistantMessage {
            return;
        }
        if !event.is_partial {
            self.0.remove(&event.turn_id);
            return;
        }
        let reply =
            self.0
                .entry(event.turn_id.clone())
                .or_insert_with(|| NormalizedTranscriptEvent {
                    text: String::new(),
                    is_partial: false,
                    ..event.clone()
                });
        reply.text.push_str(&event.text);
        reply.cursor = event.cursor;
        reply.event_id = format!("{INTERRUPTED_REPLY_EVENT_PREFIX}{}", event.event_id);
    }

    pub(crate) fn into_interrupted(self) -> impl Iterator<Item = NormalizedTranscriptEvent> {
        self.0
            .into_values()
            .filter(|reply| !reply.text.trim().is_empty())
            .map(|mut reply| {
                reply.text =
                    gent_types::bounded_text(&reply.text, gent_types::MAX_TRANSCRIPT_TEXT_BYTES)
                        .into_owned();
                reply
            })
    }
}
