//! Bounded, epoch-fenced controller transcript delta reads.

use gent_types::{HostEpoch, HostStatus, NormalizedTranscriptEvent, NormalizedTranscriptPage};

use crate::RuntimeError;

/// Bounded request for one selected-conversation controller delta batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatControllerDeltaRequest {
    pub conversation_id: String,
    pub after_cursor: u64,
    pub expected_host_epoch: HostEpoch,
    pub limit: u16,
}

/// Normalized events read under one stable daemon host epoch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatControllerDeltaPage {
    pub host_epoch: HostEpoch,
    pub events: Vec<NormalizedTranscriptEvent>,
}

/// Read boundary for a future controller stream composition.
pub trait AgentChatControllerDeltaSource {
    /// Reads the current host epoch.
    ///
    /// # Errors
    /// Returns a source read error.
    fn host_status(&self) -> Result<HostStatus, RuntimeError>;
    /// Reads one ascending, provider-neutral transcript page.
    ///
    /// # Errors
    /// Returns a source read error.
    fn transcript(
        &self,
        conversation_id: &str,
        after_cursor: u64,
        limit: u16,
    ) -> Result<NormalizedTranscriptPage, RuntimeError>;
}

/// Validates one controller delta read without retaining provider-native state.
#[derive(Clone, Copy, Debug, Default)]
pub struct AgentChatControllerDeltaReader;

impl AgentChatControllerDeltaReader {
    /// Reads a stable host-epoch bounded delta page.
    ///
    /// # Errors
    /// Returns an error for invalid requests, source failures, cursor violations, or an epoch
    /// transition during the read.
    pub fn read<S: AgentChatControllerDeltaSource>(
        source: &S,
        request: &AgentChatControllerDeltaRequest,
    ) -> Result<AgentChatControllerDeltaPage, RuntimeError> {
        validate_request(request)?;
        let before = source.host_status()?;
        if before.host_epoch != request.expected_host_epoch {
            return Err(invariant("controller delta host epoch changed"));
        }
        let page = source.transcript(
            &request.conversation_id,
            request.after_cursor,
            request.limit.clamp(1, 100),
        )?;
        let after = source.host_status()?;
        if after.host_epoch != before.host_epoch {
            return Err(invariant("controller delta host epoch changed"));
        }
        validate_page(request, &page)?;
        Ok(AgentChatControllerDeltaPage {
            host_epoch: before.host_epoch,
            events: page.events,
        })
    }
}

fn validate_request(request: &AgentChatControllerDeltaRequest) -> Result<(), RuntimeError> {
    (!request.conversation_id.trim().is_empty())
        .then_some(())
        .ok_or_else(|| invariant("controller delta requires a conversation identifier"))
}

fn validate_page(
    request: &AgentChatControllerDeltaRequest,
    page: &NormalizedTranscriptPage,
) -> Result<(), RuntimeError> {
    if page.conversation_id != request.conversation_id {
        return Err(invariant(
            "controller delta belongs to another conversation",
        ));
    }
    let mut cursor = request.after_cursor;
    for event in &page.events {
        if event.cursor <= cursor {
            return Err(invariant(
                "controller delta cursor is not strictly ascending",
            ));
        }
        cursor = event.cursor;
    }
    if page.next_after_cursor.is_some_and(|next| next <= cursor) {
        return Err(invariant("controller delta continuation does not advance"));
    }
    Ok(())
}

fn invariant(message: &str) -> RuntimeError {
    RuntimeError::Ledger(gent_ports::LedgerError::Invariant(message.into()))
}

#[cfg(test)]
#[path = "agent_chat_controller_delta_tests.rs"]
mod tests;
