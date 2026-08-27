use gent_protocol::DEFAULT_LOCAL_MODEL_ID;
use gent_types::AgentChatProvider;

pub(super) fn default_model(provider: AgentChatProvider) -> &'static str {
    match provider {
        AgentChatProvider::Claude => "haiku",
        AgentChatProvider::Codex => "default",
        AgentChatProvider::Claurst => DEFAULT_LOCAL_MODEL_ID,
    }
}
