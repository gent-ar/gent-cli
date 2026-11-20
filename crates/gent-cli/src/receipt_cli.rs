use std::path::PathBuf;

use gent_protocol::agent_chat_receipt_lookup::{
    AGENT_CHAT_RECEIPT_LOOKUP_CAPABILITY, AgentChatReceiptLookupFrame,
};
use gent_protocol::write_json_frame;
use gent_types::{AgentChatRequestId, ReceiptId};
use serde_json::Value;

use crate::local_ipc::connect_and_negotiate;

pub(crate) async fn lookup(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    receipt_id: String,
) -> Result<Value, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|capability| capability == AGENT_CHAT_RECEIPT_LOOKUP_CAPABILITY)
    {
        return Err("gentd cannot look up receipts; upgrade gentd".into());
    }
    let request_id = AgentChatRequestId(uuid::Uuid::new_v4().to_string());
    write_json_frame(
        &mut stream,
        &AgentChatReceiptLookupFrame::LookupReceipt {
            request_id: request_id.clone(),
            receipt_id: ReceiptId(receipt_id),
        },
    )
    .await?;
    let raw: Value = crate::local_ipc::read_reply(&mut stream).await?;
    match serde_json::from_value(raw)? {
        AgentChatReceiptLookupFrame::ReceiptState {
            request_id: reply,
            lookup,
        } if reply == request_id => Ok(serde_json::to_value(lookup)?),
        _ => Err("gentd returned an invalid receipt state".into()),
    }
}
