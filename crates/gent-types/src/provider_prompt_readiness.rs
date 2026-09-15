//! Immutable proof binding for releasing one held prompt after provider readiness verification.

use serde::{Deserialize, Serialize};

use crate::{
    AgentChatConversationId, AgentChatProvider, AgentChatRunId, DurableTurnPhase, ReceiptId,
};

/// Why a held prompt left the readiness hold without ever reaching a provider.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum PromptAdmissionExit {
    Failed,
    Cancelled,
}

impl PromptAdmissionExit {
    #[must_use]
    pub const fn turn_phase(self) -> DurableTurnPhase {
        match self {
            Self::Failed => DurableTurnPhase::Failed,
            Self::Cancelled => DurableTurnPhase::Cancelled,
        }
    }

    #[must_use]
    pub const fn notice(self) -> &'static str {
        match self {
            Self::Failed => "provider readiness failed",
            Self::Cancelled => "prompt cancelled before delivery",
        }
    }
}

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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderPromptReadinessFailureBinding {
    pub prompt_receipt_id: ReceiptId,
    pub conversation_id: AgentChatConversationId,
    pub run_id: AgentChatRunId,
    pub provider: AgentChatProvider,
    pub exit: PromptAdmissionExit,
    pub reason: String,
}

impl ProviderPromptReadinessFailureBinding {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        const MAX_REASON_BYTES: usize = 4 * 1024;

        ProviderPromptReadinessBinding {
            prompt_receipt_id: self.prompt_receipt_id.clone(),
            conversation_id: self.conversation_id.clone(),
            run_id: self.run_id.clone(),
            provider: self.provider,
        }
        .is_valid()
            && !self.reason.trim().is_empty()
            && self.reason.len() <= MAX_REASON_BYTES
            && !self.reason.contains('\0')
    }
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
    use super::{
        PromptAdmissionExit, ProviderPromptReadinessBinding, ProviderPromptReadinessFailureBinding,
    };
    use crate::{
        AgentChatConversationId, AgentChatProvider, AgentChatRunId, DurableTurnPhase, ReceiptId,
    };

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

    #[test]
    fn readiness_failure_binding_requires_a_bounded_reason() {
        let binding = ProviderPromptReadinessFailureBinding {
            prompt_receipt_id: ReceiptId("prompt-receipt".into()),
            conversation_id: AgentChatConversationId("conversation".into()),
            run_id: AgentChatRunId("run".into()),
            provider: AgentChatProvider::Claurst,
            exit: PromptAdmissionExit::Failed,
            reason: "transport failed".into(),
        };
        assert!(binding.is_valid());
        assert_eq!(
            PromptAdmissionExit::Failed.turn_phase(),
            DurableTurnPhase::Failed
        );
        assert_eq!(
            PromptAdmissionExit::Cancelled.turn_phase(),
            DurableTurnPhase::Cancelled
        );
        assert_eq!(
            serde_json::to_value(DurableTurnPhase::Cancelled).unwrap(),
            serde_json::json!("cancelled")
        );
        assert_eq!(
            serde_json::to_value(crate::TurnPhase::Cancelled).unwrap(),
            serde_json::json!("cancelled")
        );
        assert!(
            !ProviderPromptReadinessFailureBinding {
                reason: " ".into(),
                ..binding.clone()
            }
            .is_valid()
        );
        assert!(
            !ProviderPromptReadinessFailureBinding {
                reason: "x".repeat(4 * 1024 + 1),
                ..binding
            }
            .is_valid()
        );
    }
}
