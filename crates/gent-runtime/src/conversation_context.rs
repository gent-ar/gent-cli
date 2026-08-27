//! Bounded provider-neutral projection of one durable conversation history boundary.

use std::collections::BTreeSet;

use gent_ports::{ConversationContentReader, TranscriptLedger};
use gent_types::{
    AgentChatConversationId, ContextPolicy, ConversationContentEntry, ConversationContentPage,
    FrozenConversationContext, NormalizedTranscriptEvent, NormalizedTranscriptKind,
};
use sha2::{Digest, Sha256};

use crate::RuntimeError;

const PAGE_LIMIT: u16 = 100;
const MAX_ENTRIES: usize = 200;
const MAX_TRANSCRIPT_EVENTS: usize = 200;
const MAX_ENCODED_BYTES: usize = 512 * 1024;

/// Read-only input fixed by a durable child-run reservation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationContextRequest {
    pub conversation_id: AgentChatConversationId,
    pub context_policy: ContextPolicy,
    pub context_through_ordinal: u64,
}

/// Builds one bounded context artifact without launching, resuming, or inspecting a provider.
#[derive(Clone, Debug)]
pub struct ConversationContextArtifactService<L> {
    reader: L,
}

impl<L> ConversationContextArtifactService<L> {
    #[must_use]
    pub fn new(reader: L) -> Self {
        Self { reader }
    }
}

impl<L: ConversationContentReader + TranscriptLedger> ConversationContextArtifactService<L> {
    pub fn project_before_message(
        &self,
        conversation_id: AgentChatConversationId,
        message_id: &str,
    ) -> Result<FrozenConversationContext, RuntimeError> {
        let mut before = None;
        let mut scanned = 0usize;
        loop {
            let page =
                self.reader
                    .read_conversation_content(&conversation_id.0, before, PAGE_LIMIT)?;
            if page.conversation_id != conversation_id.0 {
                return Err(invariant(
                    "conversation context belongs to another conversation",
                ));
            }
            scanned = scanned.saturating_add(page.entries.len());
            if scanned > MAX_ENTRIES {
                return Err(invariant("conversation context exceeds entry bound"));
            }
            if let Some(entry) = page
                .entries
                .iter()
                .find(|entry| entry.message_id == message_id)
            {
                return self.project(&ConversationContextRequest {
                    conversation_id,
                    context_policy: ContextPolicy::Preserve,
                    context_through_ordinal: entry.ordinal.saturating_sub(1),
                });
            }
            before = page
                .next_before
                .map(|cursor| cursor.ordinal_for(&conversation_id.0))
                .transpose()
                .map_err(|_| invariant("conversation context cursor is invalid"))?;
            if before.is_none() {
                return Err(invariant("conversation context message is unavailable"));
            }
        }
    }

    /// Returns a chronological frozen context for preserve, or a strict empty context for clear.
    ///
    /// # Errors
    /// Returns an error when a reader violates the ordinal boundary or bounded artifact contract.
    pub fn project(
        &self,
        request: &ConversationContextRequest,
    ) -> Result<FrozenConversationContext, RuntimeError> {
        match request.context_policy {
            ContextPolicy::Clear => cleared(request),
            ContextPolicy::Preserve if request.context_through_ordinal == 0 => Ok(
                FrozenConversationContext::cleared(request.conversation_id.clone()),
            ),
            ContextPolicy::Preserve => self.preserved(request),
        }
    }

    fn preserved(
        &self,
        request: &ConversationContextRequest,
    ) -> Result<FrozenConversationContext, RuntimeError> {
        let entries = self.entries(request)?;
        let transcript_events = self.transcript(request, &entries)?;
        let artifact = FrozenConversationContext {
            conversation_id: request.conversation_id.clone(),
            context_through_ordinal: request.context_through_ordinal,
            content_digest_sha256: digest(&entries),
            transcript_digest_sha256: FrozenConversationContext::transcript_digest(
                &transcript_events,
            ),
            transcript_events,
            entries,
        };
        (serde_json::to_vec(&artifact)
            .map_err(|_| invariant("conversation context encoding failed"))?
            .len()
            <= MAX_ENCODED_BYTES)
            .then_some(artifact)
            .ok_or_else(|| invariant("conversation context exceeds byte bound"))
    }

    fn entries(
        &self,
        request: &ConversationContextRequest,
    ) -> Result<Vec<ConversationContentEntry>, RuntimeError> {
        let mut before = request
            .context_through_ordinal
            .checked_add(1)
            .ok_or_else(|| invariant("conversation context ordinal exceeds supported range"))?;
        let mut newest_first = Vec::new();
        loop {
            let page = self.reader.read_conversation_content(
                &request.conversation_id.0,
                Some(before),
                PAGE_LIMIT,
            )?;
            validate_content_page(request, before, &page, &newest_first)?;
            before = page
                .next_before
                .as_ref()
                .map(|cursor| cursor.ordinal_for(&request.conversation_id.0))
                .transpose()
                .map_err(|_| invariant("conversation context reader returned an invalid cursor"))?
                .unwrap_or(0);
            newest_first.extend(page.entries);
            if newest_first.len() > MAX_ENTRIES {
                return Err(invariant("conversation context exceeds entry bound"));
            }
            if before == 0 {
                newest_first.reverse();
                return Ok(newest_first);
            }
        }
    }

    fn transcript(
        &self,
        request: &ConversationContextRequest,
        entries: &[ConversationContentEntry],
    ) -> Result<Vec<NormalizedTranscriptEvent>, RuntimeError> {
        let turns = entries
            .iter()
            .map(|entry| entry.turn_id.as_str())
            .collect::<BTreeSet<_>>();
        let mut after = 0;
        let mut retained = Vec::new();
        loop {
            let page = self.reader.normalized_transcript_page(
                &request.conversation_id,
                after,
                PAGE_LIMIT,
            )?;
            if page.conversation_id != request.conversation_id.0 {
                return Err(invariant(
                    "conversation transcript belongs to another conversation",
                ));
            }
            let mut previous = after;
            for event in page.events {
                if event.cursor <= previous {
                    return Err(invariant("conversation transcript cursor does not advance"));
                }
                previous = event.cursor;
                if turns.contains(event.turn_id.as_str())
                    && !event.is_partial
                    && matches!(
                        event.kind,
                        NormalizedTranscriptKind::AssistantMessage
                            | NormalizedTranscriptKind::ToolActivity
                            | NormalizedTranscriptKind::Notice
                    )
                {
                    retained.push(event);
                    if retained.len() > MAX_TRANSCRIPT_EVENTS {
                        return Err(invariant("conversation transcript exceeds event bound"));
                    }
                }
            }
            match page.next_after_cursor {
                // Transcript cursors resume strictly after the last emitted event, so the
                // continuation token is that event's cursor rather than a new synthetic one.
                Some(next) if next == previous && previous > after => after = next,
                Some(_) => {
                    return Err(invariant("conversation transcript continuation is invalid"));
                }
                None => return Ok(retained),
            }
        }
    }
}

fn cleared(
    request: &ConversationContextRequest,
) -> Result<FrozenConversationContext, RuntimeError> {
    (request.context_through_ordinal == 0)
        .then(|| FrozenConversationContext::cleared(request.conversation_id.clone()))
        .ok_or_else(|| invariant("cleared conversation context must use ordinal zero"))
}

fn validate_content_page(
    request: &ConversationContextRequest,
    before: u64,
    page: &ConversationContentPage,
    prior: &[ConversationContentEntry],
) -> Result<(), RuntimeError> {
    if page.conversation_id != request.conversation_id.0 {
        return Err(invariant(
            "conversation context belongs to another conversation",
        ));
    }
    let mut previous = before;
    for entry in &page.entries {
        if entry.ordinal == 0
            || entry.ordinal >= previous
            || entry.ordinal > request.context_through_ordinal
        {
            return Err(invariant(
                "conversation context ordinal is not strictly descending",
            ));
        }
        previous = entry.ordinal;
    }
    if let Some(previous_entry) = prior.last() {
        if page
            .entries
            .first()
            .is_some_and(|entry| entry.ordinal >= previous_entry.ordinal)
        {
            return Err(invariant("conversation context pages do not advance"));
        }
    }
    match (&page.next_before, page.entries.last()) {
        (Some(cursor), Some(last))
            if cursor.ordinal_for(&request.conversation_id.0).ok() == Some(last.ordinal) =>
        {
            Ok(())
        }
        (None, _) => Ok(()),
        _ => Err(invariant("conversation context continuation is invalid")),
    }
}

fn digest(entries: &[ConversationContentEntry]) -> String {
    let mut hasher = Sha256::new();
    for entry in entries {
        hasher.update(entry.ordinal.to_be_bytes());
        hasher.update(entry.text_digest_sha256.as_bytes());
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}

fn invariant(message: &str) -> RuntimeError {
    RuntimeError::Ledger(gent_ports::LedgerError::Invariant(message.into()))
}
