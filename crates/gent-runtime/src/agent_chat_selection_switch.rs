//! Authority-gated creation of one selected child run with a frozen history boundary.

use gent_ports::AgentChatSelectionLedger;
use gent_types::{
    AgentChatConversationId, AgentChatRequestId, AgentChatRunId, AgentChatSelection,
    AgentChatSelectionSwitch, AgentChatSelectionSwitched, ContextPolicy, HostEpoch, ReceiptId,
};
use sha2::{Digest, Sha256};

use crate::{AgentChatSelectionGate, AllowAnyAgentChatSelection, RuntimeError};

/// Explicit permission to persist a selected child run.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AgentChatSelectionSwitchAuthority {
    /// Observer behavior does not claim a receipt or inspect a conversation.
    #[default]
    Observer,
    /// Reserved for the approved local writer profile.
    Approved,
}

/// Correlated request to continue from one expected current run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatSelectionSwitchRequest {
    pub request_id: AgentChatRequestId,
    pub receipt_id: ReceiptId,
    pub host_epoch: HostEpoch,
    pub conversation_id: AgentChatConversationId,
    pub parent_run_id: AgentChatRunId,
    pub selection: AgentChatSelection,
    pub context_policy: ContextPolicy,
}

/// A denied observer request or one durable selected child run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentChatSelectionSwitchResult {
    DeniedObserver,
    Switched(AgentChatSelectionSwitched),
}

/// Translates client correlation to a retry-stable child-run identity before one ledger call.
#[derive(Clone, Debug)]
pub struct AgentChatSelectionSwitchService<L, G = AllowAnyAgentChatSelection> {
    ledger: L,
    authority: AgentChatSelectionSwitchAuthority,
    selection_gate: G,
}

impl<L> AgentChatSelectionSwitchService<L, AllowAnyAgentChatSelection> {
    /// Builds an inert observer service unless the local writer is explicitly approved.
    #[must_use]
    pub fn new(ledger: L, authority: AgentChatSelectionSwitchAuthority) -> Self {
        Self::with_selection_gate(ledger, authority, AllowAnyAgentChatSelection)
    }
}

impl<L, G> AgentChatSelectionSwitchService<L, G> {
    /// Builds a service whose approved writes must pass the supplied pure selection gate.
    #[must_use]
    pub fn with_selection_gate(
        ledger: L,
        authority: AgentChatSelectionSwitchAuthority,
        selection_gate: G,
    ) -> Self {
        Self {
            ledger,
            authority,
            selection_gate,
        }
    }
}

impl<L: AgentChatSelectionLedger, G: AgentChatSelectionGate> AgentChatSelectionSwitchService<L, G> {
    /// Persists a new immutable child selection without launching or inspecting any provider.
    ///
    /// # Errors
    /// Returns an error only after approved authority reaches the durable ledger boundary.
    pub fn switch(
        &self,
        request: &AgentChatSelectionSwitchRequest,
    ) -> Result<AgentChatSelectionSwitchResult, RuntimeError> {
        if self.authority != AgentChatSelectionSwitchAuthority::Approved {
            return Ok(AgentChatSelectionSwitchResult::DeniedObserver);
        }
        if !self.selection_gate.allows(&request.selection) {
            return Err(RuntimeError::AgentChatSelectionDenied);
        }
        Ok(AgentChatSelectionSwitchResult::Switched(
            self.ledger
                .switch_agent_chat_selection(&ledger_switch(request))?,
        ))
    }
}

fn ledger_switch(request: &AgentChatSelectionSwitchRequest) -> AgentChatSelectionSwitch {
    AgentChatSelectionSwitch {
        receipt_id: request.receipt_id.clone(),
        idempotency_key: identity("receipt", &request.request_id),
        host_epoch: request.host_epoch,
        conversation_id: request.conversation_id.clone(),
        parent_run_id: request.parent_run_id.clone(),
        run_id: AgentChatRunId(identity("run", &request.request_id)),
        selection: request.selection.clone(),
        context_policy: request.context_policy,
    }
}

fn identity(kind: &str, request_id: &AgentChatRequestId) -> String {
    let mut digest = Sha256::new();
    digest.update(b"gent-agent-chat-switch-v1\0");
    digest.update(kind.as_bytes());
    digest.update([0]);
    digest.update(request_id.0.as_bytes());
    format!("agent-chat-{kind}-{:x}", digest.finalize())
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use gent_ports::{AgentChatSelectionLedger, LedgerError};
    use gent_types::{
        AgentChatRequestId, AgentChatSelection, AgentChatSelectionSwitch,
        AgentChatSelectionSwitched, ContextPolicy, ReceiptId,
    };

    use super::{AgentChatSelectionSwitchRequest, identity, ledger_switch};
    use crate::{
        AgentChatSelectionSwitchAuthority, AgentChatSelectionSwitchService,
        ExactAgentChatSelectionAllowlist, RuntimeError,
    };

    #[derive(Clone, Default)]
    struct CountingLedger(Arc<AtomicUsize>);

    impl AgentChatSelectionLedger for CountingLedger {
        fn switch_agent_chat_selection(
            &self,
            _: &AgentChatSelectionSwitch,
        ) -> Result<AgentChatSelectionSwitched, LedgerError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Err(LedgerError::Invariant("unexpected switch".into()))
        }
    }

    #[test]
    fn generated_child_identity_ignores_mutable_selection_details() {
        let request = AgentChatSelectionSwitchRequest {
            request_id: AgentChatRequestId("request-1".into()),
            receipt_id: ReceiptId("receipt-1".into()),
            host_epoch: gent_types::HostEpoch(1),
            conversation_id: gent_types::AgentChatConversationId("conversation-1".into()),
            parent_run_id: gent_types::AgentChatRunId("run-1".into()),
            selection: AgentChatSelection {
                provider: gent_types::AgentChatProvider::Claude,
                model: "haiku".into(),
                effort: gent_types::AgentChatEffort::Low,
                mode: gent_types::AgentChatMode::Ask,
            },
            context_policy: ContextPolicy::Preserve,
        };
        assert_eq!(
            ledger_switch(&request).run_id.0,
            identity("run", &request.request_id)
        );
    }

    #[test]
    fn approved_switch_rejects_disallowed_selection_before_ledger_write() {
        let ledger = CountingLedger::default();
        let service = AgentChatSelectionSwitchService::with_selection_gate(
            ledger.clone(),
            AgentChatSelectionSwitchAuthority::Approved,
            ExactAgentChatSelectionAllowlist::new([AgentChatSelection {
                provider: gent_types::AgentChatProvider::Codex,
                model: "gpt-5.6".into(),
                effort: gent_types::AgentChatEffort::Low,
                mode: gent_types::AgentChatMode::Ask,
            }]),
        );
        let request = AgentChatSelectionSwitchRequest {
            request_id: AgentChatRequestId("request-1".into()),
            receipt_id: ReceiptId("receipt-1".into()),
            host_epoch: gent_types::HostEpoch(1),
            conversation_id: gent_types::AgentChatConversationId("conversation-1".into()),
            parent_run_id: gent_types::AgentChatRunId("run-1".into()),
            selection: AgentChatSelection {
                provider: gent_types::AgentChatProvider::Claude,
                model: "haiku".into(),
                effort: gent_types::AgentChatEffort::Low,
                mode: gent_types::AgentChatMode::Ask,
            },
            context_policy: ContextPolicy::Preserve,
        };

        assert!(matches!(
            service.switch(&request),
            Err(RuntimeError::AgentChatSelectionDenied)
        ));
        assert_eq!(ledger.0.load(Ordering::Relaxed), 0);
    }
}
