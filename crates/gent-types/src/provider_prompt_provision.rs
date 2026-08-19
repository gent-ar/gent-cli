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

/// Daemon-derived package identity for a reviewed prompt-provider installation.
///
/// This is never accepted from a client frame. It is retained in the private command payload so
/// the policy selected at review time cannot change before Gent reaches the fixed npm effect.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderPromptProvisionPackageBinding {
    pub provider: String,
    pub package_name: String,
    pub version: String,
    pub integrity: String,
    pub package_policy_digest_sha256: String,
}

/// Complete daemon-built payload for a prompt-scoped provider provision command.
///
/// The public confirmation remains limited to [`ProviderPromptProvisionBinding`]. This composite
/// value is serialised only into Gent's durable command fingerprint.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderPromptProvisionCommandBinding {
    pub prompt: ProviderPromptProvisionBinding,
    /// Digest of the current daemon-issued review, retained separately from the client echo.
    pub expected_reviewed_plan_digest: String,
    pub package: ProviderPromptProvisionPackageBinding,
}

impl ProviderPromptProvisionCommandBinding {
    /// Verifies that the private policy selection belongs to the reviewed public-provider prompt.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.prompt.is_valid()
            && digest(&self.expected_reviewed_plan_digest)
            && self.package.provider == self.prompt.provider
            && package_text(&self.package.package_name, 214)
            && package_text(&self.package.version, 128)
            && self.package.integrity.starts_with("sha512-")
            && package_text(&self.package.integrity, 512)
            && digest(&self.package.package_policy_digest_sha256)
    }
}

fn package_text(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= maximum
        && value.bytes().all(|byte| byte.is_ascii_graphic())
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::{
        ProviderPromptProvisionBinding, ProviderPromptProvisionCommandBinding,
        ProviderPromptProvisionPackageBinding,
    };
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

    #[test]
    fn command_binding_requires_an_exact_daemon_package_for_the_same_provider() {
        let command = ProviderPromptProvisionCommandBinding {
            prompt: ProviderPromptProvisionBinding {
                prompt_receipt_id: ReceiptId("prompt-receipt".into()),
                conversation_id: AgentChatConversationId("conversation".into()),
                run_id: AgentChatRunId("run".into()),
                provider: "codex".into(),
                action: "install".into(),
                consent_granted: true,
                reviewed_plan_digest: "a".repeat(64),
            },
            expected_reviewed_plan_digest: "a".repeat(64),
            package: ProviderPromptProvisionPackageBinding {
                provider: "codex".into(),
                package_name: "@openai/codex".into(),
                version: "1.0.0".into(),
                integrity: "sha512-test".into(),
                package_policy_digest_sha256: "b".repeat(64),
            },
        };
        assert!(command.is_valid());
        assert!(
            !ProviderPromptProvisionCommandBinding {
                package: ProviderPromptProvisionPackageBinding {
                    provider: "claude".into(),
                    ..command.package.clone()
                },
                ..command
            }
            .is_valid()
        );
    }
}
