//! Provider-neutral selection value conversions.

use gent_protocol::DEFAULT_LOCAL_MODEL_ID;
use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider};

use super::{Effort, Mode, Provider};

pub(crate) const fn provider(value: Provider) -> AgentChatProvider {
    match value {
        Provider::Claude => AgentChatProvider::Claude,
        Provider::Codex => AgentChatProvider::Codex,
        Provider::Claurst => AgentChatProvider::Claurst,
    }
}

pub(crate) const fn effort(value: Effort) -> AgentChatEffort {
    match value {
        Effort::Low => AgentChatEffort::Low,
        Effort::Medium => AgentChatEffort::Medium,
        Effort::High => AgentChatEffort::High,
        Effort::Xhigh => AgentChatEffort::XHigh,
        Effort::Max => AgentChatEffort::Max,
        Effort::Ultra => AgentChatEffort::Ultra,
    }
}

pub(crate) const fn mode(value: Mode) -> AgentChatMode {
    match value {
        Mode::Ask => AgentChatMode::Ask,
        Mode::Plan => AgentChatMode::Plan,
        Mode::Agent => AgentChatMode::Agent,
    }
}

pub(crate) fn model(value: Provider, model: String) -> String {
    if matches!(value, Provider::Claurst) && model == "default" {
        DEFAULT_LOCAL_MODEL_ID.into()
    } else {
        model
    }
}
