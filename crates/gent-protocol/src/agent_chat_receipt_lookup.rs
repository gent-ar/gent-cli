use gent_types::{AgentChatRequestId, ReceiptId, ReceiptLookup};
use serde::{Deserialize, Serialize};

pub const AGENT_CHAT_RECEIPT_LOOKUP_CAPABILITY: &str = "agent-chat-receipt-lookup-v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AgentChatReceiptLookupFrame {
    LookupReceipt {
        request_id: AgentChatRequestId,
        receipt_id: ReceiptId,
    },
    ReceiptState {
        request_id: AgentChatRequestId,
        lookup: ReceiptLookup,
    },
}

#[cfg(test)]
mod tests {
    use super::AgentChatReceiptLookupFrame;
    use gent_types::{AgentChatRequestId, ReceiptLookup};
    use serde_json::json;

    #[test]
    fn lookup_is_a_closed_read_by_receipt_id() {
        let request: AgentChatReceiptLookupFrame = serde_json::from_value(json!({
            "type": "lookupReceipt", "body": {"requestId": "r", "receiptId": "rcpt"}
        }))
        .unwrap();
        assert!(matches!(
            request,
            AgentChatReceiptLookupFrame::LookupReceipt { .. }
        ));
        assert!(
            serde_json::from_value::<AgentChatReceiptLookupFrame>(json!({
                "type": "lookupReceipt", "body": {"requestId": "r", "receiptId": "rcpt", "text": "x"}
            }))
            .is_err()
        );
        let reply = AgentChatReceiptLookupFrame::ReceiptState {
            request_id: AgentChatRequestId("r".into()),
            lookup: ReceiptLookup::NeverSeen,
        };
        assert_eq!(
            serde_json::to_value(reply).unwrap(),
            json!({"type": "receiptState", "body": {"requestId": "r", "lookup": {"state": "neverSeen"}}})
        );
    }
}
