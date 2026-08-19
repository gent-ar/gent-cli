//! Authority-gated creation of durable agent-chat conversation identities.
//!
//! This service deliberately has no provider, process, prompt, or daemon dependency. It only
//! turns client request correlation into stable public identities before one atomic ledger call.

use gent_ports::{AgentChatLedger, AgentChatWorkspaceLedger};
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationCreated, AgentChatConversationId,
    AgentChatRequestId, AgentChatRunId, AgentChatSelection, HostEpoch, ReceiptId, WorkspaceRecord,
};
use sha2::{Digest, Sha256};

use crate::{AgentChatSelectionGate, AllowAnyAgentChatSelection, RuntimeError};

/// Explicit permission to create local agent-chat state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AgentChatConversationAuthority {
    /// Observer behavior performs no receipt claim and no database write.
    #[default]
    Observer,
    /// Reserved for the future approved single writer.
    Approved,
}

/// Client correlation and selection required to create one empty conversation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatConversationRequest {
    pub request_id: AgentChatRequestId,
    pub receipt_id: ReceiptId,
    pub host_epoch: HostEpoch,
    pub selection: AgentChatSelection,
    /// Daemon-canonicalized workspace; raw client paths never reach this pure service.
    pub workspace: WorkspaceRecord,
}

/// A denied observer request or the durable receipt and identities it created.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentChatConversationResult {
    DeniedObserver,
    Created(AgentChatConversationCreated),
}

/// Allocates retry-stable public identities and delegates their atomic ownership to the ledger.
#[derive(Clone, Debug)]
pub struct AgentChatConversationService<L, G = AllowAnyAgentChatSelection> {
    ledger: L,
    authority: AgentChatConversationAuthority,
    selection_gate: G,
}

impl<L> AgentChatConversationService<L, AllowAnyAgentChatSelection> {
    /// Builds an inert observer service unless the future writer is explicitly approved.
    #[must_use]
    pub fn new(ledger: L, authority: AgentChatConversationAuthority) -> Self {
        Self::with_selection_gate(ledger, authority, AllowAnyAgentChatSelection)
    }
}

impl<L, G> AgentChatConversationService<L, G> {
    /// Builds a service whose approved writes must pass the supplied pure selection gate.
    #[must_use]
    pub fn with_selection_gate(
        ledger: L,
        authority: AgentChatConversationAuthority,
        selection_gate: G,
    ) -> Self {
        Self {
            ledger,
            authority,
            selection_gate,
        }
    }
}

impl<L: AgentChatLedger + AgentChatWorkspaceLedger, G: AgentChatSelectionGate>
    AgentChatConversationService<L, G>
{
    /// Creates one empty conversation and its root run without starting a provider.
    ///
    /// # Errors
    /// Returns an error only after approved authority reaches the durable ledger boundary.
    pub fn create(
        &self,
        request: &AgentChatConversationRequest,
    ) -> Result<AgentChatConversationResult, RuntimeError> {
        if self.authority != AgentChatConversationAuthority::Approved {
            return Ok(AgentChatConversationResult::DeniedObserver);
        }
        if !self.selection_gate.allows(&request.selection) {
            return Err(RuntimeError::AgentChatSelectionDenied);
        }
        Ok(AgentChatConversationResult::Created(
            self.ledger.create_agent_chat_conversation_in_workspace(
                &ledger_create(request),
                &request.workspace,
            )?,
        ))
    }
}

fn ledger_create(request: &AgentChatConversationRequest) -> AgentChatConversationCreate {
    AgentChatConversationCreate {
        receipt_id: request.receipt_id.clone(),
        idempotency_key: stable_identity("receipt", &request.request_id),
        host_epoch: request.host_epoch,
        conversation_id: AgentChatConversationId(stable_identity(
            "conversation",
            &request.request_id,
        )),
        run_id: AgentChatRunId(stable_identity("run", &request.request_id)),
        selection: request.selection.clone(),
    }
}

fn stable_identity(kind: &str, request_id: &AgentChatRequestId) -> String {
    let mut digest = Sha256::new();
    digest.update(b"gent-agent-chat-v1\0");
    digest.update(kind.as_bytes());
    digest.update([0]);
    digest.update(request_id.0.as_bytes());
    format!("{kind}-{:x}", digest.finalize())
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::{AgentChatConversationRequest, ledger_create};
    use gent_ports::{AgentChatLedger, AgentChatWorkspaceLedger, LedgerError};
    use gent_types::{
        AgentChatConversationCreate, AgentChatConversationCreated, AgentChatEffort, AgentChatMode,
        AgentChatProvider, AgentChatRequestId, AgentChatSelection, HostEpoch, ReceiptId,
        WorkspaceRecord,
    };

    use crate::{
        AgentChatConversationAuthority, AgentChatConversationService,
        ExactAgentChatSelectionAllowlist, RuntimeError,
    };

    #[derive(Clone, Default)]
    struct CountingLedger(Arc<AtomicUsize>);

    impl AgentChatLedger for CountingLedger {
        fn create_agent_chat_conversation(
            &self,
            _: &AgentChatConversationCreate,
        ) -> Result<AgentChatConversationCreated, LedgerError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Err(LedgerError::Invariant("unexpected create".into()))
        }
    }

    impl AgentChatWorkspaceLedger for CountingLedger {
        fn create_agent_chat_conversation_in_workspace(
            &self,
            _: &AgentChatConversationCreate,
            _: &WorkspaceRecord,
        ) -> Result<AgentChatConversationCreated, LedgerError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Err(LedgerError::Invariant("unexpected workspace create".into()))
        }

        fn agent_chat_workspace_for_run(
            &self,
            _: &str,
            _: &str,
        ) -> Result<WorkspaceRecord, LedgerError> {
            Err(LedgerError::Invariant("unexpected workspace read".into()))
        }
    }

    fn request(request_id: &str) -> AgentChatConversationRequest {
        AgentChatConversationRequest {
            request_id: AgentChatRequestId(request_id.into()),
            receipt_id: ReceiptId("receipt-1".into()),
            host_epoch: HostEpoch(1),
            selection: AgentChatSelection {
                provider: AgentChatProvider::Claude,
                model: "haiku".into(),
                effort: AgentChatEffort::Low,
                mode: AgentChatMode::Ask,
            },
            workspace: WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/workspace".into(),
            },
        }
    }

    #[test]
    fn public_identities_are_stable_for_correlation_not_selection_content() {
        let first = ledger_create(&request("request-1"));
        let mut changed = request("request-1");
        changed.selection.model = "another-model".into();
        let second = ledger_create(&changed);
        assert_eq!(first.conversation_id, second.conversation_id);
        assert_eq!(first.run_id, second.run_id);
        assert_eq!(first.idempotency_key, second.idempotency_key);
    }

    #[test]
    fn approved_create_rejects_disallowed_selection_before_ledger_write() {
        let ledger = CountingLedger::default();
        let service = AgentChatConversationService::with_selection_gate(
            ledger.clone(),
            AgentChatConversationAuthority::Approved,
            ExactAgentChatSelectionAllowlist::new([AgentChatSelection {
                provider: AgentChatProvider::Codex,
                model: "gpt-5.6".into(),
                effort: AgentChatEffort::Low,
                mode: AgentChatMode::Ask,
            }]),
        );

        assert!(matches!(
            service.create(&request("rejected")),
            Err(RuntimeError::AgentChatSelectionDenied)
        ));
        assert_eq!(ledger.0.load(Ordering::Relaxed), 0);
    }
}
