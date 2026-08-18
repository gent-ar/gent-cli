//! Parent-fenced switch request construction for the terminal reducer.

use gent_types::{AgentChatSelection, ContextPolicy};

use super::state::{UiEffect, UiRequest};

pub(super) fn request(
    conversation_id: Option<String>,
    parent_run_id: Option<String>,
    selection: AgentChatSelection,
    context_policy: ContextPolicy,
) -> Result<UiEffect, &'static str> {
    let conversation_id =
        conversation_id.ok_or("Select a conversation before switching its selection.")?;
    let parent_run_id = parent_run_id
        .filter(|run_id| !run_id.is_empty())
        .ok_or("Run status is unavailable; refusing to guess a switch parent.")?;
    Ok(UiEffect::Request(UiRequest::Switch {
        conversation_id,
        parent_run_id,
        selection,
        context_policy,
    }))
}
