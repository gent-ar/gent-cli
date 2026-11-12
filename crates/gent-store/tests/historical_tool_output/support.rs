use super::*;

pub(super) fn prepared() -> (SqliteLedger, String) {
    let ledger = SqliteLedger::in_memory().unwrap();
    let turn = prepare_conversation(&ledger, "a");
    (ledger, turn)
}

pub(super) fn prepare_conversation(ledger: &SqliteLedger, name: &str) -> String {
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId(format!("conversation-receipt-{name}")),
                idempotency_key: format!("conversation-key-{name}"),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId(format!("conversation-{name}")),
                run_id: AgentChatRunId(format!("run-{name}")),
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
            request_id: AgentChatRequestId(format!("prompt-{name}")),
            receipt_id: ReceiptId(format!("prompt-receipt-{name}")),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId(format!("conversation-{name}")),
            disposition: AgentChatPromptDisposition::Send,
            text: "hello".into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    ledger
        .claim_run_lease(&gent_ports::RunLease {
            run_id: format!("run-{name}"),
            coordinator_id: "daemon-a".into(),
            host_epoch: HostEpoch(1),
        })
        .unwrap();
    ledger
        .save_run_session_binding(&gent_ports::RunSessionBinding {
            run_id: format!("run-{name}"),
            provider_session_id: format!("private-session-{name}"),
        })
        .unwrap();
    prompt.message.turn_id
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
