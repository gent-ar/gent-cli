//! Exact durable command identity for a daemon-derived prompt-provider installation.

use gent_types::{Command, HostEpoch, ProviderPromptProvisionCommandBinding, ReceiptId};

/// Builds the only command identity that may reserve a reviewed provider install for one prompt.
///
/// # Panics
/// Panics only when the fixed binding type cannot serialize to JSON.
#[must_use]
pub fn prompt_provider_provision_command(
    receipt_id: ReceiptId,
    idempotency_key: String,
    host_epoch: HostEpoch,
    binding: &ProviderPromptProvisionCommandBinding,
) -> Command {
    Command {
        receipt_id,
        idempotency_key,
        host_epoch,
        kind: "providerPromptProvision".into(),
        payload: serde_json::to_value(binding).expect("prompt provision binding serializes"),
    }
}

#[cfg(test)]
mod tests {
    use super::prompt_provider_provision_command;
    use gent_types::{
        AgentChatConversationId, AgentChatRunId, HostEpoch, ProviderPromptProvisionBinding,
        ProviderPromptProvisionCommandBinding, ProviderPromptProvisionPackageBinding, ReceiptId,
    };

    #[test]
    fn command_binds_only_the_daemon_derived_prompt_provision_identity() {
        let binding = ProviderPromptProvisionCommandBinding {
            prompt: ProviderPromptProvisionBinding {
                prompt_receipt_id: ReceiptId("prompt".into()),
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
        let command = prompt_provider_provision_command(
            ReceiptId("provision".into()),
            "key".into(),
            HostEpoch(1),
            &binding,
        );
        assert_eq!(command.kind, "providerPromptProvision");
        assert_eq!(command.payload, serde_json::to_value(binding).unwrap());
    }
}
