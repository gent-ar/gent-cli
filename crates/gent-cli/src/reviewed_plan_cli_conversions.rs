use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider, ContextPolicy};

use super::{ContextArgument, EffortArgument, ModeArgument, ProviderArgument};

impl From<ProviderArgument> for AgentChatProvider {
    fn from(value: ProviderArgument) -> Self {
        match value {
            ProviderArgument::Claude => Self::Claude,
            ProviderArgument::Codex => Self::Codex,
            ProviderArgument::Claurst => Self::Claurst,
        }
    }
}

impl From<EffortArgument> for AgentChatEffort {
    fn from(value: EffortArgument) -> Self {
        match value {
            EffortArgument::Low => Self::Low,
            EffortArgument::Medium => Self::Medium,
            EffortArgument::High => Self::High,
            EffortArgument::Xhigh => Self::XHigh,
            EffortArgument::Max => Self::Max,
            EffortArgument::Ultra => Self::Ultra,
        }
    }
}

impl From<ModeArgument> for AgentChatMode {
    fn from(value: ModeArgument) -> Self {
        match value {
            ModeArgument::Ask => Self::Ask,
            ModeArgument::Plan => Self::Plan,
            ModeArgument::Agent => Self::Agent,
        }
    }
}

impl From<ContextArgument> for ContextPolicy {
    fn from(value: ContextArgument) -> Self {
        match value {
            ContextArgument::Preserve => Self::Preserve,
            ContextArgument::Clear => Self::Clear,
        }
    }
}
