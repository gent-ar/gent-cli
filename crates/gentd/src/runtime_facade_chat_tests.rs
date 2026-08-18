//! Integration checks for durable-chat read composition, separate from observer tests.

use gent_protocol::{
    AGENT_CHAT_CONVERSATIONS_CAPABILITY, AGENT_CHAT_TRANSCRIPT_CAPABILITY,
    AgentChatConversationFrame, AgentChatIntentFrame, AgentChatTranscriptFrame,
    ORCHESTRATION_CAPABILITY,
};
use gent_runtime::catalog::{
    declared_capabilities_with_agent_chat, validate_observed_capabilities,
};
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRequestId, AgentChatSelection,
    ReceiptId,
};

use crate::{CompatibilityAssessment, api::RuntimeApi, build_runtime};

#[test]
fn durable_chat_authority_advertises_and_serves_only_normalized_read_models() {
    let directory = tempfile::tempdir().unwrap();
    let capabilities = crate::transport::observed_capabilities(true, false, false);
    assert_eq!(
        validate_observed_capabilities(&capabilities).unwrap(),
        declared_capabilities_with_agent_chat(true)
    );
    assert!(
        capabilities
            .0
            .contains(&AGENT_CHAT_CONVERSATIONS_CAPABILITY.into())
    );
    assert!(
        capabilities
            .0
            .contains(&AGENT_CHAT_TRANSCRIPT_CAPABILITY.into())
    );
    assert!(capabilities.0.contains(&ORCHESTRATION_CAPABILITY.into()));
    let runtime = build_runtime(
        directory.path(),
        &capabilities,
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let created = runtime
        .agent_chat_intent(AgentChatIntentFrame::CreateConversation {
            request_id: AgentChatRequestId("request-1".into()),
            receipt_id: ReceiptId("receipt-1".into()),
            selection: AgentChatSelection {
                provider: AgentChatProvider::Codex,
                model: "gpt-5.6".into(),
                effort: AgentChatEffort::High,
                mode: AgentChatMode::Agent,
            },
        })
        .unwrap();
    let [
        AgentChatIntentFrame::Created {
            conversation_id, ..
        },
    ] = created.as_slice()
    else {
        panic!("durable chat profile must create one conversation");
    };
    let summary = runtime
        .agent_chat_conversation(AgentChatConversationFrame::SummaryRequest {
            conversation_id: conversation_id.0.clone(),
        })
        .unwrap();
    assert!(matches!(
        summary,
        AgentChatConversationFrame::Summary(value)
            if value.selection.provider == AgentChatProvider::Codex
    ));
    let transcript = runtime
        .agent_chat_transcript(AgentChatTranscriptFrame::PageRequest {
            conversation_id: conversation_id.0.clone(),
            after_cursor: None,
            limit: 20,
        })
        .unwrap();
    assert!(matches!(
        transcript,
        AgentChatTranscriptFrame::Page(value)
            if value.conversation_id == conversation_id.0 && value.events.is_empty()
    ));
}
