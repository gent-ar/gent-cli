use gent_protocol::{
    DependencyProvider, PromptProviderProvisionFrame, PromptProviderProvisionState,
};
use gent_types::{
    AgentChatProvider, Command, Event, ProviderPromptProvisionCommandBinding, Receipt,
};

pub(super) struct Derived {
    pub(super) receipt_id: gent_types::ReceiptId,
    pub(super) idempotency_key: String,
    pub(super) host_epoch: gent_types::HostEpoch,
    pub(super) binding: ProviderPromptProvisionCommandBinding,
}

pub(super) fn public_provider(provider: AgentChatProvider) -> Result<DependencyProvider, String> {
    match provider {
        AgentChatProvider::Claude => Ok(DependencyProvider::Claude),
        AgentChatProvider::Codex => Ok(DependencyProvider::Codex),
        AgentChatProvider::Claurst => {
            Err("Claurst uses its private bridge and cannot enter public npm provision".into())
        }
    }
}

pub(super) fn public_provider_name(provider: &str) -> Result<DependencyProvider, String> {
    provider
        .parse()
        .map_err(|_| "daemon-derived provider binding is invalid".into())
}

pub(super) fn event(command: &Command, suffix: &str, kind: &str) -> Event {
    Event {
        cursor: 0,
        event_id: format!("{}:prompt-provision:{suffix}", command.receipt_id.0),
        receipt_id: command.receipt_id.clone(),
        host_epoch: command.host_epoch,
        kind: kind.into(),
        payload: command.payload.clone(),
    }
}

pub(super) fn result(
    receipt: Receipt,
    binding: &ProviderPromptProvisionCommandBinding,
    state: PromptProviderProvisionState,
) -> PromptProviderProvisionFrame {
    PromptProviderProvisionFrame::Result {
        receipt,
        prompt_receipt_id: binding.prompt.prompt_receipt_id.clone(),
        conversation_id: binding.prompt.conversation_id.clone(),
        run_id: binding.prompt.run_id.clone(),
        state,
    }
}
