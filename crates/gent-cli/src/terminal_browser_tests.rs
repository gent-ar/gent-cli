use gent_types::{AgentChatPromptDelivery, ConversationListItem};

use super::{delivery_notice, initial_view};

#[test]
fn prompt_delivery_notice_never_claims_a_provider_started() {
    assert!(delivery_notice(AgentChatPromptDelivery::Queued).contains("queued"));
    assert!(delivery_notice(AgentChatPromptDelivery::AwaitingReadiness).contains("preparing"));
    assert!(delivery_notice(AgentChatPromptDelivery::AwaitingProvider).contains("thinking"));
}

#[tokio::test]
async fn initial_view_does_not_infer_activity_or_content_without_capabilities() {
    let index = vec![ConversationListItem {
        conversation_id: "conversation-1".into(),
        run_count: 1,
    }];
    assert!(initial_view(&index, &[], None, true).await.is_none());
}
