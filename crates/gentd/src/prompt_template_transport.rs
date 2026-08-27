use gent_protocol::{PROMPT_TEMPLATES_CAPABILITY, PromptTemplateFrame, write_json_frame};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::io::AsyncWrite;

use crate::{api::RuntimeApi, transport::write_error};

pub(crate) async fn dispatch<S, R>(
    stream: &mut S,
    runtime: &R,
    capabilities: &CapabilitySet,
    raw: &Value,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    R: RuntimeApi,
{
    if !capabilities
        .0
        .iter()
        .any(|item| item == PROMPT_TEMPLATES_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<PromptTemplateFrame>(raw.clone()) else {
        return Ok(false);
    };
    if frame.validate().is_err() || !client_request(&frame) {
        write_error(
            stream,
            "invalidPromptTemplate",
            "prompt template request is invalid",
        )
        .await?;
        return Ok(true);
    }
    match runtime.prompt_templates(frame) {
        Ok(reply) => write_json_frame(stream, &reply).await?,
        Err(message) => write_error(stream, "promptTemplateRejected", &message).await?,
    }
    Ok(true)
}

fn client_request(frame: &PromptTemplateFrame) -> bool {
    matches!(
        frame,
        PromptTemplateFrame::Create { .. }
            | PromptTemplateFrame::List { .. }
            | PromptTemplateFrame::Get { .. }
            | PromptTemplateFrame::Delete { .. }
            | PromptTemplateFrame::Render { .. }
    )
}
