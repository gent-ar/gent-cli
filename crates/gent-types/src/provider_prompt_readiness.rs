//! Immutable proof binding for releasing one held prompt after provider readiness verification.

use serde::{Deserialize, Serialize};

use crate::{AgentChatConversationId, AgentChatProvider, AgentChatRunId, ReceiptId};

/// Daemon-derived identity of one held prompt whose selected provider is verified ready.
///
/// This excludes executable paths, package details, and provider-native session values. The
/// durable command and terminal event carry this exact value as their payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderPromptReadinessBinding {
    pub prompt_receipt_id: ReceiptId,
    pub conversation_id: AgentChatConversationId,
    pub run_id: AgentChatRunId,
    pub provider: AgentChatProvider,
}

impl ProviderPromptReadinessBinding {
    /// Ensures all durable identities are present before a ledger command is constructed.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        [
            &self.prompt_receipt_id.0,
            &self.conversation_id.0,
            &self.run_id.0,
        ]
        .into_iter()
        .all(|value| !value.trim().is_empty() && !value.contains('\0'))
    }
}

#[cfg(test)]
mod tests {
    use super::ProviderPromptReadinessBinding;
    use crate::{AgentChatConversationId, AgentChatProvider, AgentChatRunId, ReceiptId};

    #[test]
    fn readiness_binding_requires_nonempty_durable_identity() {
        let binding = ProviderPromptReadinessBinding {
            prompt_receipt_id: ReceiptId("prompt-receipt".into()),
            conversation_id: AgentChatConversationId("conversation".into()),
            run_id: AgentChatRunId("run".into()),
            provider: AgentChatProvider::Codex,
        };
        assert!(binding.is_valid());
        assert!(
            !ProviderPromptReadinessBinding {
                run_id: AgentChatRunId("\0".into()),
                ..binding
            }
            .is_valid()
        );
    }
}
