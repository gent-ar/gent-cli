use gent_protocol::agent_chat_receipt_lookup::{
    AGENT_CHAT_RECEIPT_LOOKUP_CAPABILITY, AgentChatReceiptLookupFrame,
};
use gent_protocol::write_json_frame;
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
        .any(|capability| capability == AGENT_CHAT_RECEIPT_LOOKUP_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<AgentChatReceiptLookupFrame>(raw.clone()) else {
        return Ok(false);
    };
    let AgentChatReceiptLookupFrame::LookupReceipt {
        request_id,
        receipt_id,
    } = frame
    else {
        write_error(
            stream,
            "invalidReceiptLookup",
            "receipt state frames are server-only",
        )
        .await?;
        return Ok(true);
    };
    let Some(port) = runtime.agent_chat_permission_port() else {
        write_error(
            stream,
            "receiptLookupUnavailable",
            "receipt lookup is unavailable",
        )
        .await?;
        return Ok(true);
    };
    match port.lookup_receipt(&receipt_id) {
        Ok(lookup) => {
            write_json_frame(
                stream,
                &AgentChatReceiptLookupFrame::ReceiptState { request_id, lookup },
            )
            .await?;
        }
        Err(message) => write_error(stream, "receiptLookupFailed", &message).await?,
    }
    Ok(true)
}
