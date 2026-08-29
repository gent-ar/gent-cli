//! Authority-gated creation of a new conversation seeded from another's prior messages.

use gent_ports::AgentChatForkLedger;
use gent_types::{AgentChatConversationId, AgentChatFork, AgentChatForked, AgentChatRunId};
use sha2::{Digest, Sha256};

use crate::RuntimeError;

/// Explicit permission to fork durable agent-chat conversations.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AgentChatForkAuthority {
    /// Observer behavior performs no receipt claim and no database write.
    #[default]
    Observer,
    /// Reserved for the future approved single writer.
    Approved,
}

/// A denied observer request or the durable receipt and identities it created.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentChatForkResult {
    DeniedObserver,
    Forked(AgentChatForked),
}

/// Allocates retry-stable public identities and delegates their atomic ownership to the ledger.
#[derive(Clone, Debug)]
pub struct AgentChatForkService<L> {
    ledger: L,
    authority: AgentChatForkAuthority,
}

impl<L> AgentChatForkService<L> {
    #[must_use]
    pub fn new(ledger: L, authority: AgentChatForkAuthority) -> Self {
        Self { ledger, authority }
    }
}

impl<L: AgentChatForkLedger> AgentChatForkService<L> {
    /// Copies a source conversation's messages up to the fork point into a new conversation.
    ///
    /// # Errors
    /// Returns an error only after approved authority reaches the durable ledger boundary.
    pub fn fork(&self, fork: &AgentChatFork) -> Result<AgentChatForkResult, RuntimeError> {
        if self.authority != AgentChatForkAuthority::Approved {
            return Ok(AgentChatForkResult::DeniedObserver);
        }
        let conversation_id = AgentChatConversationId(stable_identity("conversation", fork));
        let run_id = AgentChatRunId(stable_identity("run", fork));
        Ok(AgentChatForkResult::Forked(
            self.ledger
                .fork_agent_chat_conversation(fork, &conversation_id, &run_id)?,
        ))
    }
}

fn stable_identity(kind: &str, fork: &AgentChatFork) -> String {
    let mut digest = Sha256::new();
    digest.update(b"gent-agent-chat-fork-v1\0");
    digest.update(kind.as_bytes());
    digest.update([0]);
    digest.update(fork.request_id.0.as_bytes());
    format!("{kind}-{:x}", digest.finalize())
}

#[cfg(test)]
mod tests {
    use super::{AgentChatForkAuthority, AgentChatForkResult, AgentChatForkService};
    use gent_store::SqliteLedger;
    use gent_types::{
        AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatFork,
        AgentChatMode, AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider,
        AgentChatRequestId, AgentChatRunId, AgentChatSelection, HostEpoch, ReceiptId,
    };

    fn selection() -> AgentChatSelection {
        AgentChatSelection {
            provider: AgentChatProvider::Claude,
            model: "claude-sonnet".into(),
            effort: AgentChatEffort::Medium,
            mode: AgentChatMode::Agent,
        }
    }

    #[test]
    fn observer_authority_denies_every_fork_request() {
        let ledger = SqliteLedger::in_memory().unwrap();
        let service = AgentChatForkService::new(ledger, AgentChatForkAuthority::Observer);
        let result = service
            .fork(&AgentChatFork {
                request_id: AgentChatRequestId("request-1".into()),
                receipt_id: ReceiptId("receipt-1".into()),
                host_epoch: HostEpoch(1),
                source_conversation_id: AgentChatConversationId("conversation-1".into()),
                fork_through_message_id: "message-1".into(),
            })
            .unwrap();
        assert_eq!(result, AgentChatForkResult::DeniedObserver);
    }

    #[test]
    fn approved_authority_copies_messages_into_a_new_conversation() {
        use gent_ports::{AgentChatPromptLedger, AgentChatWorkspaceLedger};

        let ledger = SqliteLedger::in_memory().unwrap();
        ledger
            .create_agent_chat_conversation_in_workspace(
                &AgentChatConversationCreate {
                    receipt_id: ReceiptId("create-receipt".into()),
                    idempotency_key: "create-key".into(),
                    host_epoch: HostEpoch(1),
                    conversation_id: AgentChatConversationId("conversation-1".into()),
                    run_id: AgentChatRunId("run-1".into()),
                    selection: selection(),
                },
                &gent_types::WorkspaceRecord {
                    workspace_id: "workspace-1".into(),
                    canonical_path: "/workspace-1".into(),
                },
            )
            .unwrap();
        let saved = ledger
            .save_agent_chat_prompt(&AgentChatPromptCreate {
                request_id: AgentChatRequestId("prompt-request".into()),
                receipt_id: ReceiptId("prompt-receipt".into()),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation-1".into()),
                disposition: AgentChatPromptDisposition::Send,
                attachment_ids: vec![],
                tool_source_ids: vec![],
                text: "hello".into(),
            })
            .unwrap();
        let service = AgentChatForkService::new(ledger, AgentChatForkAuthority::Approved);
        let result = service
            .fork(&AgentChatFork {
                request_id: AgentChatRequestId("fork-request".into()),
                receipt_id: ReceiptId("fork-receipt".into()),
                host_epoch: HostEpoch(1),
                source_conversation_id: AgentChatConversationId("conversation-1".into()),
                fork_through_message_id: saved.message.message_id.clone(),
            })
            .unwrap();
        let AgentChatForkResult::Forked(forked) = result else {
            unreachable!()
        };
        assert_ne!(forked.conversation_id.0, "conversation-1");
    }
}
