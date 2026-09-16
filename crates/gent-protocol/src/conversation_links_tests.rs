use super::{
    CONVERSATION_LINKS_CAPABILITY, ConversationLinkRequestError, bounded_wait_seconds,
    is_conversation_link_frame, validate_conversation_link_request,
};
use crate::AgentChatIntentFrame;
use gent_types::{AgentChatConversationId, AgentChatRequestId, ReceiptId};
use serde_json::json;

fn conversation(value: &str) -> AgentChatConversationId {
    AgentChatConversationId(value.into())
}

fn create(label: &str, prompt: Option<&str>) -> AgentChatIntentFrame {
    AgentChatIntentFrame::CreateLinkedConversation {
        request_id: AgentChatRequestId("request-1".into()),
        receipt_id: ReceiptId("receipt-1".into()),
        parent_conversation_id: conversation("parent-1"),
        workspace_path: None,
        selection: None,
        label: label.into(),
        prompt: prompt.map(str::to_owned),
        origin_tool_use_id: None,
    }
}

#[test]
fn the_capability_is_its_own_versioned_family() {
    assert_eq!(CONVERSATION_LINKS_CAPABILITY, "conversation-links-v1");
    assert!(is_conversation_link_frame(&create("child", None)));
    assert!(!is_conversation_link_frame(
        &AgentChatIntentFrame::Subscribe {
            request_id: AgentChatRequestId("request-1".into()),
            conversation_id: conversation("parent-1"),
            after_cursor: 0,
        }
    ));
}

#[test]
fn creating_a_child_requires_a_bounded_label_and_a_nonempty_prompt() {
    assert!(validate_conversation_link_request(&create("child", Some("go"))).is_ok());
    assert!(validate_conversation_link_request(&create("child", None)).is_ok());
    assert_eq!(
        validate_conversation_link_request(&create("  ", None)),
        Err(ConversationLinkRequestError::InvalidLabel)
    );
    assert_eq!(
        validate_conversation_link_request(&create("child", Some("   "))),
        Err(ConversationLinkRequestError::EmptyMessage)
    );
}

#[test]
fn a_conversation_may_not_address_or_await_itself() {
    let send = AgentChatIntentFrame::SendToConversation {
        request_id: AgentChatRequestId("request-1".into()),
        receipt_id: ReceiptId("receipt-1".into()),
        from_conversation_id: conversation("parent-1"),
        target_conversation_id: conversation("parent-1"),
        message: "hello".into(),
    };
    assert_eq!(
        validate_conversation_link_request(&send),
        Err(ConversationLinkRequestError::SelfTargeted)
    );
    let wait = AgentChatIntentFrame::WaitForConversations {
        request_id: AgentChatRequestId("request-1".into()),
        from_conversation_id: conversation("parent-1"),
        conversation_ids: vec![conversation("child-1"), conversation("parent-1")],
        timeout_seconds: 10,
    };
    assert_eq!(
        validate_conversation_link_request(&wait),
        Err(ConversationLinkRequestError::SelfTargeted)
    );
    assert_eq!(
        ConversationLinkRequestError::SelfTargeted.code(),
        "conversationSelfTargeted"
    );
}

#[test]
fn a_wait_names_between_one_and_sixteen_conversations() {
    let wait = |count: usize| AgentChatIntentFrame::WaitForConversations {
        request_id: AgentChatRequestId("request-1".into()),
        from_conversation_id: conversation("parent-1"),
        conversation_ids: (0..count)
            .map(|index| conversation(&format!("child-{index}")))
            .collect(),
        timeout_seconds: 10,
    };
    assert!(validate_conversation_link_request(&wait(1)).is_ok());
    assert!(validate_conversation_link_request(&wait(16)).is_ok());
    assert_eq!(
        validate_conversation_link_request(&wait(0)),
        Err(ConversationLinkRequestError::InvalidTargetCount)
    );
    assert_eq!(
        validate_conversation_link_request(&wait(17)),
        Err(ConversationLinkRequestError::InvalidTargetCount)
    );
}

#[test]
fn a_wait_is_clamped_instead_of_refused() {
    assert_eq!(bounded_wait_seconds(30), 30);
    assert_eq!(bounded_wait_seconds(u32::MAX), 900);
}

#[test]
fn creating_a_child_round_trips_its_optional_defaults() {
    assert_eq!(
        serde_json::to_value(create("Build the parser", Some("start"))).unwrap(),
        json!({
            "type": "createLinkedConversation",
            "body": {
                "requestId": "request-1", "receiptId": "receipt-1",
                "parentConversationId": "parent-1", "label": "Build the parser",
                "prompt": "start"
            }
        })
    );
}
