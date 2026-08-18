//! Bounded terminal-side defaults for the provider-neutral selection controls.

use gent_types::{AgentChatProvider, AgentChatSelection};

pub(super) fn default_model(provider: AgentChatProvider) -> &'static str {
    if provider == AgentChatProvider::Claude {
        "haiku"
    } else {
        "default"
    }
}

pub(super) fn next_model(selection: &AgentChatSelection) -> &'static str {
    match selection.provider {
        AgentChatProvider::Claude if selection.model == "haiku" => "sonnet",
        AgentChatProvider::Claude => "haiku",
        AgentChatProvider::Codex if selection.model == "default" => "gpt-5.6",
        AgentChatProvider::Claurst if selection.model == "default" => "claurst-main",
        AgentChatProvider::Codex | AgentChatProvider::Claurst => "default",
    }
}
