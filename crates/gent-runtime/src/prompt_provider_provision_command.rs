//! Exact durable command identity for a daemon-derived prompt-provider installation.

use gent_types::{Command, HostEpoch, ProviderPromptProvisionBinding, ReceiptId};

/// Builds the only command identity that may reserve a reviewed provider install for one prompt.
///
/// # Panics
/// Panics only when the fixed binding type cannot serialize to JSON.
#[must_use]
pub fn prompt_provider_provision_command(
    receipt_id: ReceiptId,
    idempotency_key: String,
    host_epoch: HostEpoch,
    binding: &ProviderPromptProvisionBinding,
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
        ReceiptId,
    };

    #[test]
    fn command_binds_only_the_daemon_derived_prompt_provision_identity() {
        let binding = ProviderPromptProvisionBinding {
            prompt_receipt_id: ReceiptId("prompt".into()),
            conversation_id: AgentChatConversationId("conversation".into()),
            run_id: AgentChatRunId("run".into()),
            provider: "codex".into(),
            action: "install".into(),
            consent_granted: true,
            reviewed_plan_digest: "a".repeat(64),
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
