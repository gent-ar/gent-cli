use gent_protocol::AgentChatIntentFrame;
use gent_types::{
    AgentChatConversationId, AgentChatPromptDelivery, AgentChatRequestId, AgentChatRunId,
    HostEpoch, Receipt, ReceiptId, ReceiptStatus,
};

use super::super::valid_reply;

#[test]
fn accepted_prompt_must_retain_the_requested_conversation_identity() {
    let request = AgentChatIntentFrame::SendPrompt {
        request_id: AgentChatRequestId("request-1".into()),
        receipt_id: ReceiptId("receipt-1".into()),
        conversation_id: AgentChatConversationId("conversation-1".into()),
        text: "hello".into(),
        attachment_ids: Vec::new(),
    };
    let mut reply = AgentChatIntentFrame::Accepted {
        request_id: AgentChatRequestId("request-1".into()),
        receipt: Receipt {
            receipt_id: ReceiptId("receipt-1".into()),
            idempotency_key: "retry-1".into(),
            status: ReceiptStatus::Accepted,
            host_epoch: HostEpoch(1),
        },
        conversation_id: AgentChatConversationId("other-conversation".into()),
        run_id: AgentChatRunId("run-1".into()),
        turn_id: "turn-1".into(),
        delivery: AgentChatPromptDelivery::AwaitingProvider,
    };
    assert!(!valid_reply(&request, &reply));
    let AgentChatIntentFrame::Accepted {
        conversation_id, ..
    } = &mut reply
    else {
        unreachable!();
    };
    *conversation_id = AgentChatConversationId("conversation-1".into());
    assert!(valid_reply(&request, &reply));
}
