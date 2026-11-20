use gent_ports::{
    AgentChatPromptLedger, ClaurstDrainBatch, ClaurstFactValue, ClaurstNormalizedFact,
    TranscriptLedger,
};
use gent_runtime::Coordinator;
use gent_testkit::FakePrivateClaurstBridge;
use gent_types::{
    AgentChatConversationId, AgentChatFailureCode, AgentChatPromptCreate,
    AgentChatPromptDisposition, AgentChatRequestId, CapabilitySet, HostEpoch,
    NormalizedProviderEvent, NormalizedTranscriptKind, ProviderFailureClassification, ReceiptId,
};

use crate::private_claurst_ingress::PrivateClaurstIngress;
use crate::private_claurst_ingress_tests::{binding, checkpoint, prepared_ledger, start_request};

#[tokio::test]
async fn a_claurst_failure_reaches_the_transcript_with_its_typed_code() {
    let ledger = prepared_ledger();
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("request-failure".into()),
            receipt_id: ReceiptId("receipt-failure".into()),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            disposition: AgentChatPromptDisposition::Send,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: "run out of tokens".into(),
        })
        .unwrap();
    let bridge = FakePrivateClaurstBridge::default();
    let binding = binding();
    bridge.push_start_binding(binding.clone());
    bridge.push_batch(ClaurstDrainBatch {
        facts: vec![ClaurstNormalizedFact {
            source_id: binding.source_id.clone(),
            cursor: 1,
            value: ClaurstFactValue::Event(NormalizedProviderEvent::ProviderFailure {
                classification: ProviderFailureClassification::OutputLimit,
                message: "Claurst exhausted its output limit before completing the turn.".into(),
                resets_at: None,
            }),
        }],
        permissions: vec![],
        checkpoint: Some(checkpoint(1)),
        session_binding: Some(binding.clone()),
        terminal: Some(gent_ports::ClaurstTerminal::Failed {
            classification: gent_ports::ClaurstFailureClassification::Internal,
        }),
    });
    let mut ingress = PrivateClaurstIngress::new(
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger.clone(),
        bridge,
        "daemon-a".into(),
    );
    let mut request = start_request();
    request.turn_id = saved.message.turn_id;
    ingress.start(request, HostEpoch(1)).await.unwrap();
    ingress
        .drain(&binding.source_id, HostEpoch(1))
        .await
        .unwrap();
    let page = ledger
        .normalized_transcript_page(&AgentChatConversationId("conversation-a".into()), 0, 8)
        .unwrap();
    let notice = page
        .events
        .iter()
        .find(|event| event.kind == NormalizedTranscriptKind::Notice)
        .unwrap();
    assert_eq!(
        notice.failure_code,
        Some(AgentChatFailureCode::ProviderOutputLimit)
    );
    assert_eq!(
        serde_json::to_value(notice).unwrap()["failureCode"],
        "providerOutputLimit"
    );
}
