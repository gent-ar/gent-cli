use gent_types::AgentChatPromptDelivery;

use super::{delivery_notice, initial::initial_view};

#[test]
fn prompt_delivery_notice_never_claims_a_provider_started() {
    assert!(delivery_notice(AgentChatPromptDelivery::Queued).contains("queued"));
    assert!(delivery_notice(AgentChatPromptDelivery::AwaitingReadiness).contains("preparing"));
    assert!(delivery_notice(AgentChatPromptDelivery::AwaitingProvider).contains("thinking"));
}

#[tokio::test]
async fn initial_view_does_not_infer_activity_or_content_without_capabilities() {
    assert!(
        initial_view(Some("conversation-1".into()), &[], None, true)
            .await
            .is_none()
    );
}
