//! Durable recovery from a failed provider compaction without provider access.

use gent_core::AgentChatCompactionEffect;
use gent_ports::AgentChatSelectionLedger;
use gent_types::{
    AgentChatConversationId, AgentChatRequestId, AgentChatRunId, AgentChatSelection, ContextPolicy,
    HostEpoch, ReceiptId,
};
use sha2::{Digest, Sha256};

use crate::{
    AgentChatSelectionSwitchAuthority, AgentChatSelectionSwitchRequest,
    AgentChatSelectionSwitchResult, AgentChatSelectionSwitchService, RuntimeError,
};

/// Explicit permission to create a compaction-recovery child run.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AgentChatCompactionRecoveryAuthority {
    /// Observer behavior cannot inspect a compaction effect or write a recovery.
    #[default]
    Observer,
    /// Reserved for the approved daemon-owned lifecycle composition.
    Approved,
}

/// Durable identifiers needed to recover one failed compaction effect.
///
/// `source_event_id` must be the stable identity of a prior persisted normalized source fact.
/// It is hashed before it becomes a receipt or run identity and never becomes provider input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatCompactionRecoveryRequest {
    pub source_event_id: String,
    /// Cursor assigned to the normalized failed-compaction fact before recovery is considered.
    pub source_cursor: u64,
    pub host_epoch: HostEpoch,
    pub conversation_id: AgentChatConversationId,
    pub parent_run_id: AgentChatRunId,
    pub selection: AgentChatSelection,
}

/// A denied observer request, an inapplicable effect, or one durable fresh-session child run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentChatCompactionRecoveryResult {
    DeniedObserver,
    Ignored,
    Recovered(gent_types::AgentChatSelectionSwitched),
}

/// Turns the pure recovery effect into the existing atomic selection-switch transaction.
///
/// The selected child inherits a frozen history ordinal from the ledger. Because child runs have
/// no inherited session binding, a future runner must start a fresh provider-native session.
#[derive(Clone, Debug)]
pub struct AgentChatCompactionRecoveryService<L> {
    switches: AgentChatSelectionSwitchService<L>,
    authority: AgentChatCompactionRecoveryAuthority,
}

impl<L> AgentChatCompactionRecoveryService<L> {
    /// Builds an inert service unless daemon-owned authority is explicitly approved.
    #[must_use]
    pub fn new(ledger: L, authority: AgentChatCompactionRecoveryAuthority) -> Self {
        let switch_authority = match authority {
            AgentChatCompactionRecoveryAuthority::Observer => {
                AgentChatSelectionSwitchAuthority::Observer
            }
            AgentChatCompactionRecoveryAuthority::Approved => {
                AgentChatSelectionSwitchAuthority::Approved
            }
        };
        Self {
            switches: AgentChatSelectionSwitchService::new(ledger, switch_authority),
            authority,
        }
    }
}

impl<L: AgentChatSelectionLedger> AgentChatCompactionRecoveryService<L> {
    /// Persists one retry-stable child only for a pure `RecoverFromFrozenLedger` effect.
    ///
    /// # Errors
    /// Returns an error only when approved authority reaches the durable switch ledger.
    pub fn apply(
        &self,
        request: &AgentChatCompactionRecoveryRequest,
        effect: &AgentChatCompactionEffect,
    ) -> Result<AgentChatCompactionRecoveryResult, RuntimeError> {
        if self.authority != AgentChatCompactionRecoveryAuthority::Approved {
            return Ok(AgentChatCompactionRecoveryResult::DeniedObserver);
        }
        let AgentChatCompactionEffect::RecoverFromFrozenLedger {
            event_id,
            turn_id,
            source_cursor,
        } = effect
        else {
            return Ok(AgentChatCompactionRecoveryResult::Ignored);
        };
        validate(request, event_id, turn_id, *source_cursor)?;
        let switched = self.switches.switch(&switch_request(request, turn_id))?;
        match switched {
            AgentChatSelectionSwitchResult::Switched(result) => {
                Ok(AgentChatCompactionRecoveryResult::Recovered(result))
            }
            AgentChatSelectionSwitchResult::DeniedObserver => Err(invariant(
                "compaction authority disagrees with selection authority",
            )),
        }
    }
}

fn validate(
    request: &AgentChatCompactionRecoveryRequest,
    event_id: &str,
    turn_id: &str,
    source_cursor: u64,
) -> Result<(), RuntimeError> {
    if request.source_event_id.trim().is_empty()
        || request.source_event_id.len() > 256
        || turn_id.trim().is_empty()
        || request.source_cursor == 0
        || request.source_cursor != source_cursor
        || request.source_event_id != event_id
    {
        return Err(invariant(
            "compaction recovery source identity and cursor must match the failed turn",
        ));
    }
    Ok(())
}

fn switch_request(
    request: &AgentChatCompactionRecoveryRequest,
    turn_id: &str,
) -> AgentChatSelectionSwitchRequest {
    let seed = format!(
        "{}\0{}\0{}\0{}\0{}",
        request.conversation_id.0,
        request.parent_run_id.0,
        turn_id,
        request.source_cursor,
        request.source_event_id
    );
    AgentChatSelectionSwitchRequest {
        request_id: AgentChatRequestId(identity("request", &seed)),
        receipt_id: ReceiptId(identity("receipt", &seed)),
        host_epoch: request.host_epoch,
        conversation_id: request.conversation_id.clone(),
        parent_run_id: request.parent_run_id.clone(),
        selection: request.selection.clone(),
        context_policy: ContextPolicy::Preserve,
    }
}

fn identity(kind: &str, seed: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"gent-agent-chat-compaction-recovery-v1\0");
    digest.update(kind.as_bytes());
    digest.update([0]);
    digest.update(seed.as_bytes());
    format!("agent-chat-compaction-{kind}-{:x}", digest.finalize())
}

fn invariant(message: &str) -> RuntimeError {
    RuntimeError::Ledger(gent_ports::LedgerError::Invariant(message.into()))
}
