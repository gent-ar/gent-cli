use super::*;

pub(super) fn prepared() -> (SqliteLedger, String) {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation-a".into()),
                run_id: AgentChatRunId("run-a".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Codex,
                    model: "gpt".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace-a".into(),
                canonical_path: "/workspace-a".into(),
            },
        )
        .unwrap();
    let prompt = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-a".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            disposition: AgentChatPromptDisposition::Send,
            text: "hello".into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    ledger
        .claim_run_lease(&gent_ports::RunLease {
            run_id: "run-a".into(),
            coordinator_id: "daemon-a".into(),
            host_epoch: HostEpoch(1),
        })
        .unwrap();
    ledger
        .save_run_session_binding(&gent_ports::RunSessionBinding {
            run_id: "run-a".into(),
            provider_session_id: "private-session".into(),
        })
        .unwrap();
    (ledger, prompt.message.turn_id)
}

pub(super) fn batch(turn_id: String) -> NormalizedSessionBatch {
    NormalizedSessionBatch {
        coordinator_id: "daemon-a".into(),
        conversation_id: "conversation-a".into(),
        run_id: "run-a".into(),
        turn_id: turn_id.clone(),
        host_epoch: HostEpoch(1),
        lifecycle_event_id: "lifecycle-a".into(),
        lifecycle: NormalizedSessionLifecycle::Event {
            event: NormalizedProviderEvent::TurnStarted {
                turn_id: turn_id.clone(),
            },
        },
        transcript: Some(NormalizedTranscriptAppend {
            event_id: "transcript-a".into(),
            turn_id: turn_id.clone(),
            run_id: "run-a".into(),
            kind: NormalizedTranscriptKind::AssistantMessage,
            text: "working".into(),
            is_partial: true,
        }),
        activity_event_id: Some("activity-a".into()),
        activity: Some(ConversationActivityFact::TurnStarted {
            scope: ConversationActivityScope {
                conversation_id: "conversation-a".into(),
                run_id: "run-a".into(),
                turn_id,
                host_epoch: HostEpoch(1),
                cursor: 0,
            },
        }),
    }
}
