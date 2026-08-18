//! Unadvertised controller snapshot composition for reconnecting chat clients.
//!
//! This deliberately has no wire types or daemon bootstrap. A future daemon composition supplies
//! the source port after it has negotiated authority and capabilities.

use gent_types::{
    AgentChatConversationDetail, AgentChatSelection, ConversationStatus, HostEpoch, HostStatus,
    NormalizedTranscriptPage,
};

use crate::RuntimeError;

/// Bounded read requested by a controller restoring one conversation projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatControllerSnapshotRequest {
    pub conversation_id: String,
    pub after_cursor: Option<u64>,
    pub transcript_limit: u16,
    pub expected_selection: Option<AgentChatSelection>,
    pub include_status: bool,
}

/// One self-consistent-enough durable controller projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatControllerSnapshot {
    pub host_epoch: HostEpoch,
    pub detail: AgentChatConversationDetail,
    pub transcript: NormalizedTranscriptPage,
    pub status: Option<ConversationStatus>,
}

/// Read boundary owned by a future daemon controller composition.
///
/// The source must return normalized, bounded values only. It must never surface provider-native
/// sessions, credentials, raw provider payloads, or transport frames.
pub trait AgentChatControllerSnapshotSource {
    /// Reads the current host status and fence epoch.
    ///
    /// # Errors
    /// Returns a source read error.
    fn host_status(&self) -> Result<HostStatus, RuntimeError>;
    /// Reads one normalized conversation and its visible run hierarchy.
    ///
    /// # Errors
    /// Returns a source read error.
    fn conversation_detail(
        &self,
        conversation_id: &str,
    ) -> Result<AgentChatConversationDetail, RuntimeError>;
    /// Reads one ascending normalized transcript page.
    ///
    /// # Errors
    /// Returns a source read error.
    fn transcript(
        &self,
        conversation_id: &str,
        after_cursor: Option<u64>,
        limit: u16,
    ) -> Result<NormalizedTranscriptPage, RuntimeError>;
    /// Reads an optional non-content lifecycle projection for the selected conversation.
    ///
    /// # Errors
    /// Returns a source read error.
    fn conversation_status(
        &self,
        conversation_id: &str,
    ) -> Result<ConversationStatus, RuntimeError>;
}

/// Pure validation plus a fence-rechecked controller snapshot read.
#[derive(Clone, Copy, Debug, Default)]
pub struct AgentChatControllerSnapshotBuilder;

impl AgentChatControllerSnapshotBuilder {
    /// Reads a selected conversation while rejecting a host-epoch transition between reads.
    ///
    /// # Errors
    /// Returns an error when the source fails, its values violate public invariants, or the host
    /// epoch changes before the read completes.
    pub fn read<S: AgentChatControllerSnapshotSource>(
        source: &S,
        request: &AgentChatControllerSnapshotRequest,
    ) -> Result<AgentChatControllerSnapshot, RuntimeError> {
        validate_request(request)?;
        let before = source.host_status()?;
        let detail = source.conversation_detail(&request.conversation_id)?;
        let transcript = source.transcript(
            &request.conversation_id,
            request.after_cursor,
            request.transcript_limit.clamp(1, 100),
        )?;
        let status = request
            .include_status
            .then(|| source.conversation_status(&request.conversation_id))
            .transpose()?;
        let after = source.host_status()?;
        Self::build(request, &before, detail, transcript, status, &after)
    }

    /// Validates values already read through a source port without performing I/O.
    ///
    /// # Errors
    /// Returns an error when selection, transcript cursor, conversation, or epoch invariants fail.
    pub fn build(
        request: &AgentChatControllerSnapshotRequest,
        before: &HostStatus,
        detail: AgentChatConversationDetail,
        transcript: NormalizedTranscriptPage,
        status: Option<ConversationStatus>,
        after: &HostStatus,
    ) -> Result<AgentChatControllerSnapshot, RuntimeError> {
        validate_request(request)?;
        if before.host_epoch != after.host_epoch {
            return Err(invariant("host epoch changed during controller snapshot"));
        }
        if detail.summary.conversation_id != request.conversation_id
            || transcript.conversation_id != request.conversation_id
        {
            return Err(invariant(
                "controller snapshot contains another conversation",
            ));
        }
        if status
            .as_ref()
            .is_some_and(|status| status.conversation_id != request.conversation_id)
        {
            return Err(invariant(
                "controller snapshot status belongs to another conversation",
            ));
        }
        validate_selection(&detail.summary.selection)?;
        for run in &detail.runs {
            if run.run_id.trim().is_empty() {
                return Err(invariant("controller snapshot has an empty run identifier"));
            }
            validate_selection(&run.selection)?;
        }
        if request
            .expected_selection
            .as_ref()
            .is_some_and(|selection| selection != &detail.summary.selection)
        {
            return Err(invariant("controller snapshot selection changed"));
        }
        validate_page(request.after_cursor, &transcript)?;
        Ok(AgentChatControllerSnapshot {
            host_epoch: before.host_epoch,
            detail,
            transcript,
            status,
        })
    }
}

fn validate_request(request: &AgentChatControllerSnapshotRequest) -> Result<(), RuntimeError> {
    if request.conversation_id.trim().is_empty() {
        return Err(invariant(
            "controller snapshot requires a conversation identifier",
        ));
    }
    Ok(())
}

fn validate_selection(selection: &AgentChatSelection) -> Result<(), RuntimeError> {
    if selection.model.trim().is_empty() {
        return Err(invariant(
            "controller snapshot selection has an empty model",
        ));
    }
    Ok(())
}

fn validate_page(
    after_cursor: Option<u64>,
    transcript: &NormalizedTranscriptPage,
) -> Result<(), RuntimeError> {
    let mut previous = after_cursor.unwrap_or(0);
    for event in &transcript.events {
        if event.cursor <= previous {
            return Err(invariant(
                "controller snapshot transcript cursor is not strictly ascending",
            ));
        }
        previous = event.cursor;
    }
    if transcript
        .next_after_cursor
        .is_some_and(|cursor| cursor <= previous)
    {
        return Err(invariant(
            "controller snapshot transcript continuation does not advance",
        ));
    }
    Ok(())
}

fn invariant(message: &str) -> RuntimeError {
    RuntimeError::Ledger(gent_ports::LedgerError::Invariant(message.into()))
}

#[cfg(test)]
#[path = "agent_chat_controller_snapshot_tests.rs"]
mod tests;
