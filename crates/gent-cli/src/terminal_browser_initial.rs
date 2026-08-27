use std::path::PathBuf;

use gent_protocol::{AGENT_CHAT_TRANSCRIPT_CAPABILITY, CONVERSATION_STATUS_CAPABILITY};
use gent_types::ConversationListItem;

use crate::terminal;

use super::read_view;

pub(super) async fn initial_view(
    index: &[ConversationListItem],
    capabilities: &[String],
    data_dir: Option<PathBuf>,
    no_autostart: bool,
) -> Option<terminal::ConversationView> {
    let conversation_id = index.first()?.conversation_id.clone();
    capabilities
        .iter()
        .any(|capability| {
            capability == CONVERSATION_STATUS_CAPABILITY
                || capability == AGENT_CHAT_TRANSCRIPT_CAPABILITY
        })
        .then(|| read_view(data_dir, no_autostart, conversation_id, capabilities))?
        .await
        .ok()
}
