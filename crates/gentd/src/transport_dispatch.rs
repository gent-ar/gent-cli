use gent_protocol::{ATTACHMENTS_CAPABILITY, AttachmentFrame};
use serde_json::Value;
use tokio::io::AsyncWrite;

use super::ExtensionSupport;
use crate::{
    activity_transport, agent_chat_checkpoint_transport, agent_chat_command_transport,
    agent_chat_conversation_config_transport, agent_chat_permission_transport,
    agent_chat_projection_transport, agent_chat_read_transport, agent_chat_sessions_transport,
    agent_chat_side_question_transport, agent_chat_transport, api::RuntimeApi,
    automation_transport, conversation_transport, forge_transport, goal_transport,
    local_model_transport, orchestration_transport, permission_policy_transport,
    prompt_provider_provision_transport, prompt_template_transport, provider_auth_transport,
    provider_readiness_transport, reviewed_plan_transport, runtime_facade::model_catalog,
    runtime_maintenance_transport, runtime_update_transport, workspace_documents_transport,
    workspace_git_transport,
};

pub(super) async fn dispatch_extension<S, R>(
    stream: &mut S,
    runtime: &R,
    extensions: &ExtensionSupport,
    raw: &Value,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    R: RuntimeApi,
{
    let capabilities = &extensions.0;
    if agent_chat_projection_transport::dispatch(stream, runtime, capabilities, raw).await?
        || agent_chat_read_transport::dispatch(stream, runtime, capabilities, raw).await?
        || automation_transport::dispatch(stream, runtime, capabilities, raw).await?
        || agent_chat_sessions_transport::dispatch(stream, runtime, capabilities, raw).await?
        || forge_transport::dispatch(stream, runtime, capabilities, raw).await?
        || provider_readiness_transport::dispatch(stream, runtime, capabilities, raw).await?
        || prompt_provider_provision_transport::dispatch(stream, runtime, capabilities, raw).await?
        || agent_chat_transport::dispatch(stream, runtime, capabilities, raw).await?
        || agent_chat_command_transport::dispatch(stream, runtime, capabilities, raw).await?
        || agent_chat_permission_transport::dispatch(stream, runtime, capabilities, raw).await?
        || permission_policy_transport::dispatch(stream, runtime, capabilities, raw).await?
        || agent_chat_conversation_config_transport::dispatch(stream, runtime, capabilities, raw)
            .await?
        || agent_chat_checkpoint_transport::dispatch(stream, runtime, capabilities, raw).await?
        || agent_chat_side_question_transport::dispatch(stream, runtime, capabilities, raw).await?
        || provider_auth_transport::dispatch(stream, runtime, capabilities, raw).await?
        || reviewed_plan_transport::dispatch(stream, runtime, capabilities, raw).await?
        || orchestration_transport::dispatch(stream, runtime, capabilities, raw).await?
        || goal_transport::dispatch(stream, runtime, capabilities, raw).await?
        || prompt_template_transport::dispatch(stream, runtime, capabilities, raw).await?
        || workspace_documents_transport::dispatch(stream, runtime, capabilities, raw).await?
        || workspace_git_transport::dispatch(stream, runtime, capabilities, raw).await?
        || conversation_transport::dispatch(stream, runtime, capabilities, raw).await?
        || activity_transport::dispatch(stream, runtime, capabilities, raw).await?
        || runtime_update_transport::dispatch(stream, runtime, capabilities, raw).await?
        || runtime_maintenance_transport::dispatch(stream, runtime, capabilities, raw).await?
        || local_model_transport::dispatch(stream, runtime, capabilities, raw).await?
        || model_catalog::transport::dispatch(stream, runtime, capabilities, raw).await?
    {
        return Ok(true);
    }
    if extensions.supports(ATTACHMENTS_CAPABILITY) {
        if let Ok(frame) = serde_json::from_value::<AttachmentFrame>(raw.clone()) {
            return crate::attachment_transport::write(stream, runtime, frame).await;
        }
    }
    Ok(false)
}
