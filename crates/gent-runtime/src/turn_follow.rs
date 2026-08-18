//! Bounded, epoch-fenced read-only projection for following one terminal turn.

use gent_ports::{TurnFollowPage, TurnFollowReader};
use gent_types::{HostEpoch, NormalizedTranscriptEvent, TurnTerminal};

use crate::RuntimeError;

const MAX_TURN_FOLLOW_EVENTS: u16 = 100;

/// Bounded read request for one exact conversation/run/turn tuple.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnFollowRequest {
    pub conversation_id: String,
    pub run_id: String,
    pub turn_id: String,
    pub after_cursor: u64,
    pub expected_host_epoch: HostEpoch,
    pub limit: u16,
}

/// Validated durable deltas and an optional settled terminal record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnFollowRead {
    pub host_epoch: HostEpoch,
    pub events: Vec<NormalizedTranscriptEvent>,
    pub next_after_cursor: Option<u64>,
    pub terminal: Option<TurnTerminal>,
}

/// Reads one exact turn without provider access, process control, or mutable authority.
#[derive(Clone, Copy, Debug, Default)]
pub struct TurnFollowService;

impl TurnFollowService {
    /// Reads a stable epoch-fenced page and only settles a durable terminal after all pages.
    ///
    /// # Errors
    /// Returns an invariant error for scope, cursor, epoch, or terminal ordering violations.
    pub fn read<S: TurnFollowReader>(
        source: &S,
        request: &TurnFollowRequest,
    ) -> Result<TurnFollowRead, RuntimeError> {
        validate_request(request)?;
        let before = source.turn_follow_host_epoch()?;
        if before != request.expected_host_epoch {
            return Err(invariant("turn follow host epoch changed"));
        }
        let page = source.turn_follow_page(
            &request.conversation_id,
            &request.run_id,
            &request.turn_id,
            request.after_cursor,
            request.limit.clamp(1, MAX_TURN_FOLLOW_EVENTS),
        )?;
        if source.turn_follow_host_epoch()? != before {
            return Err(invariant("turn follow host epoch changed"));
        }
        validate_page(request, &page)?;
        let cursor = page
            .events
            .last()
            .map_or(request.after_cursor, |event| event.cursor);
        let terminal =
            (page.turn.phase.is_terminal() && page.next_after_cursor.is_none()).then(|| {
                TurnTerminal {
                    conversation_id: page.turn.conversation_id.clone(),
                    run_id: page.turn.run_id.clone(),
                    turn_id: page.turn.turn_id.clone(),
                    phase: page.turn.phase,
                    cursor,
                }
            });
        Ok(TurnFollowRead {
            host_epoch: before,
            events: page.events,
            next_after_cursor: page.next_after_cursor,
            terminal,
        })
    }
}

fn validate_request(request: &TurnFollowRequest) -> Result<(), RuntimeError> {
    if [&request.conversation_id, &request.run_id, &request.turn_id]
        .into_iter()
        .any(|value| value.trim().is_empty() || value.contains('\0'))
    {
        return Err(invariant("turn follow requires exact public identities"));
    }
    Ok(())
}

fn validate_page(request: &TurnFollowRequest, page: &TurnFollowPage) -> Result<(), RuntimeError> {
    if page.turn.conversation_id != request.conversation_id
        || page.turn.run_id != request.run_id
        || page.turn.turn_id != request.turn_id
    {
        return Err(invariant(
            "turn follow page belongs to another durable turn",
        ));
    }
    let mut cursor = request.after_cursor;
    for event in &page.events {
        if event.cursor <= cursor
            || event.run_id != request.run_id
            || event.turn_id != request.turn_id
        {
            return Err(invariant("turn follow events violate cursor or scope"));
        }
        cursor = event.cursor;
    }
    if page
        .next_after_cursor
        .is_some_and(|next| next < cursor || (next == cursor && page.events.is_empty()))
    {
        return Err(invariant("turn follow continuation does not advance"));
    }
    if page.events.len() > usize::from(request.limit.clamp(1, MAX_TURN_FOLLOW_EVENTS)) {
        return Err(invariant("turn follow page exceeds its bounded limit"));
    }
    Ok(())
}

fn invariant(message: &str) -> RuntimeError {
    RuntimeError::Ledger(gent_ports::LedgerError::Invariant(message.into()))
}

#[cfg(test)]
#[path = "turn_follow_tests.rs"]
mod tests;
