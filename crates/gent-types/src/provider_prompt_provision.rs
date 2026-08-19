//! Immutable command binding for one reviewed provider installation that releases one prompt.

use serde::{Deserialize, Serialize};

use crate::{AgentChatConversationId, AgentChatRunId, ReceiptId};

/// All client-confirmed fields for one daemon-issued, prompt-scoped install review.
///
/// The daemon derives the provider and plan first. This value binds a confirmation to one held
/// prompt without carrying package names, executable paths, or provider-native session values.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderPromptProvisionBinding {
    pub prompt_receipt_id: ReceiptId,
    pub conversation_id: AgentChatConversationId,
    pub run_id: AgentChatRunId,
    pub provider: String,
    pub action: String,
    pub consent_granted: bool,
    pub reviewed_plan_digest: String,
}

impl ProviderPromptProvisionBinding {
    /// Verifies bounded public provider/action labels and an exact SHA-256 review digest.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        matches!(self.provider.as_str(), "claude" | "codex")
            && matches!(self.action.as_str(), "install" | "update")
            && !self.prompt_receipt_id.0.trim().is_empty()
            && !self.conversation_id.0.trim().is_empty()
            && !self.run_id.0.trim().is_empty()
            && self.reviewed_plan_digest.len() == 64
            && self
                .reviewed_plan_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    }
}

#[cfg(test)]
mod tests {
    use super::ProviderPromptProvisionBinding;
    use crate::{AgentChatConversationId, AgentChatRunId, ReceiptId};

    #[test]
    fn only_public_provider_install_bindings_are_valid() {
        let binding = ProviderPromptProvisionBinding {
            prompt_receipt_id: ReceiptId("prompt-receipt".into()),
            conversation_id: AgentChatConversationId("conversation".into()),
            run_id: AgentChatRunId("run".into()),
            provider: "codex".into(),
            action: "install".into(),
            consent_granted: true,
            reviewed_plan_digest: "a".repeat(64),
        };
        assert!(binding.is_valid());
        assert!(
            !ProviderPromptProvisionBinding {
                provider: "claurst".into(),
                ..binding
            }
            .is_valid()
        );
    }
}
