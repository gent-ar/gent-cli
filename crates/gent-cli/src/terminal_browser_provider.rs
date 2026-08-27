use std::path::PathBuf;

use gent_protocol::LocalModelInstallState;
use gent_types::{AgentChatConversationDetail, AgentChatProvider, PermissionDecisionRequest};

use crate::local_models_cli;

pub(super) async fn model_state(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    detail: Option<&AgentChatConversationDetail>,
    capabilities: &[String],
) -> Result<Option<LocalModelInstallState>, String> {
    let model_id = detail
        .filter(|value| value.summary.selection.provider == AgentChatProvider::Claurst)
        .filter(|_| {
            capabilities
                .iter()
                .any(|value| value == gent_protocol::LOCAL_MODELS_CAPABILITY)
        })
        .map(|value| value.summary.selection.model.clone());
    match model_id {
        Some(model_id) => local_models_cli::status(data_dir, no_autostart, model_id)
            .await
            .map(Some)
            .map_err(|error| error.to_string()),
        None => Ok(None),
    }
}

pub(super) async fn local_model_ids(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    capabilities: &[String],
) -> Vec<String> {
    if !capabilities
        .iter()
        .any(|value| value == gent_protocol::LOCAL_MODELS_CAPABILITY)
    {
        return Vec::new();
    }
    local_models_cli::list(data_dir, no_autostart)
        .await
        .map(|models| models.into_iter().map(|model| model.id).collect())
        .unwrap_or_default()
}

pub(super) async fn pending_permission(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: &str,
    detail: Option<&AgentChatConversationDetail>,
    capabilities: &[String],
) -> Result<Option<PermissionDecisionRequest>, String> {
    if !capabilities
        .iter()
        .any(|value| value == gent_protocol::AGENT_CHAT_PERMISSIONS_CAPABILITY)
    {
        return Ok(None);
    }
    match detail {
        Some(detail) => crate::permissions_cli::agent_chat::pending(
            data_dir,
            no_autostart,
            conversation_id.into(),
            detail.current_run_id.clone(),
        )
        .await
        .map_err(|error| error.to_string()),
        None => Ok(None),
    }
}
