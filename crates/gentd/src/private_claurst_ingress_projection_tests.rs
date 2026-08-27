use gent_ports::{
    AgentChatPromptLedger, ClaurstDrainBatch, ClaurstFactValue, ClaurstNormalizedFact,
    ClaurstPermissionRequest, ConversationActivityLedger, PendingPermissionLedger,
    TranscriptLedger,
};
use gent_runtime::Coordinator;
use gent_testkit::FakePrivateClaurstBridge;
use gent_types::{
    AgentChatConversationId, AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatRequestId,
    AgentChatRunId, CapabilitySet, HostEpoch, NormalizedProviderEvent, NormalizedTranscriptKind,
    PermissionCategory, ReceiptId, ToolPhase, TurnPhase,
};

use crate::private_claurst_ingress::PrivateClaurstIngress;
use crate::private_claurst_ingress_tests::{binding, checkpoint, prepared_ledger, start_request};

#[tokio::test]
async fn started_source_projects_output_into_the_shared_conversation_transcript() {
    let ledger = prepared_ledger();
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("request-output".into()),
            receipt_id: ReceiptId("receipt-output".into()),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            disposition: AgentChatPromptDisposition::Send,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: "render output".into(),
        })
        .unwrap();
    let bridge = FakePrivateClaurstBridge::default();
    let binding = binding();
    bridge.push_start_binding(binding.clone());
    bridge.push_batch(ClaurstDrainBatch {
        facts: vec![ClaurstNormalizedFact {
            source_id: binding.source_id.clone(),
            cursor: 1,
            value: ClaurstFactValue::Event(NormalizedProviderEvent::Output {
                text: "local Claurst reply".into(),
                is_partial: true,
            }),
        }],
        permissions: vec![],
        checkpoint: Some(checkpoint(1)),
        session_binding: Some(binding.clone()),
        terminal: Some(gent_ports::ClaurstTerminal::Completed),
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
    assert_eq!(page.events.len(), 2);
    assert_eq!(page.events[0].kind, NormalizedTranscriptKind::UserMessage);
    assert_eq!(page.events[0].text, "render output");
    assert!(!page.events[0].is_partial);
    assert_eq!(
        page.events[1].kind,
        NormalizedTranscriptKind::AssistantMessage
    );
    assert_eq!(page.events[1].text, "local Claurst reply");
    assert!(page.events[1].is_partial);
    assert!(
        ledger
            .read_conversation_activity_page("conversation-a", "run-a", 0, 8)
            .unwrap()
            .facts
            .is_empty()
    );
}

#[tokio::test]
async fn started_source_projects_context_usage_into_shared_activity() {
    let ledger = prepared_ledger();
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("request-context".into()),
            receipt_id: ReceiptId("receipt-context".into()),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            disposition: AgentChatPromptDisposition::Send,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: "measure context".into(),
        })
        .unwrap();
    let bridge = FakePrivateClaurstBridge::default();
    let binding = binding();
    bridge.push_start_binding(binding.clone());
    bridge.push_batch(ClaurstDrainBatch {
        facts: vec![ClaurstNormalizedFact {
            source_id: binding.source_id.clone(),
            cursor: 1,
            value: ClaurstFactValue::Event(NormalizedProviderEvent::ContextUsage {
                used_tokens: 12,
                window_tokens: Some(24),
            }),
        }],
        permissions: vec![],
        checkpoint: Some(checkpoint(1)),
        session_binding: Some(binding.clone()),
        terminal: Some(gent_ports::ClaurstTerminal::Completed),
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
    assert!(matches!(
        ledger
            .read_conversation_activity_page("conversation-a", "run-a", 0, 8)
            .unwrap()
            .facts
            .as_slice(),
        [gent_types::ConversationActivityFact::ContextUsage {
            used_tokens: 12,
            window_tokens: Some(24),
            ..
        }]
    ));
}

#[tokio::test]
async fn started_source_projects_pending_permission_into_shared_activity() {
    let ledger = prepared_ledger();
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("request-permission".into()),
            receipt_id: ReceiptId("receipt-permission".into()),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            disposition: AgentChatPromptDisposition::Send,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: "request permission".into(),
        })
        .unwrap();
    let bridge = FakePrivateClaurstBridge::default();
    let binding = binding();
    bridge.push_start_binding(binding.clone());
    bridge.push_batch(ClaurstDrainBatch {
        facts: vec![],
        permissions: vec![ClaurstPermissionRequest {
            request_id: "permission-1".into(),
            tool_use_id: "tool-1".into(),
            tool_name: "write_file".into(),
            category: PermissionCategory::Edit,
        }],
        checkpoint: Some(checkpoint(0)),
        session_binding: Some(binding.clone()),
        terminal: None,
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
    assert!(
        ledger
            .pending_permission(
                &AgentChatConversationId("conversation-a".into()),
                &AgentChatRunId("run-a".into())
            )
            .unwrap()
            .is_some()
    );
    let facts = ledger
        .read_conversation_activity_page("conversation-a", "run-a", 0, 8)
        .unwrap()
        .facts;
    assert!(facts.iter().any(|fact| matches!(
        fact,
        gent_types::ConversationActivityFact::RootPhase {
            phase: TurnPhase::WaitingPermission,
            ..
        }
    )));
    assert!(facts.iter().any(|fact| matches!(
        fact,
        gent_types::ConversationActivityFact::ToolActivity {
            activity: gent_types::ToolActivity {
                phase: ToolPhase::WaitingPermission,
                ..
            },
            ..
        }
    )));
}
