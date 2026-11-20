pub use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatPromptLedger, AgentChatQueuedPromptLedger,
    AgentChatWorkspaceLedger, AttachmentLedger, ConversationActivityLedger, ConversationLedger,
    Ledger,
};
pub use gent_store::SqliteLedger;
pub use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatProvider,
    AgentChatRequestId, AgentChatRunId, AgentChatSelection, AttachmentMetadata, AttachmentState,
    AttachmentTransfer, Command, DurableTurnPhase, HostEpoch, ReceiptId, WorkspaceRecord,
};
pub use serde_json::{Value, json};

pub const DAEMON: &str = "daemon-a";
pub const EPOCH: HostEpoch = HostEpoch(1);

pub fn conversation() -> AgentChatConversationId {
    AgentChatConversationId("conversation-1".into())
}

pub fn ledger() -> SqliteLedger {
    ledger_for(AgentChatProvider::Claude)
}

pub fn ledger_for(provider: AgentChatProvider) -> SqliteLedger {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create-receipt".into()),
                idempotency_key: "create-key".into(),
                host_epoch: EPOCH,
                conversation_id: conversation(),
                run_id: AgentChatRunId("run-1".into()),
                selection: AgentChatSelection {
                    provider,
                    model: "sonnet".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    ledger
}

pub fn stage(ledger: &SqliteLedger, attachment_id: &str) {
    let uploading = AttachmentTransfer {
        metadata: AttachmentMetadata {
            attachment_id: attachment_id.into(),
            display_name: "diagram.png".into(),
            media_type: "image/png".into(),
            byte_len: 5,
            digest_sha256: "b".repeat(64),
            storage_key: format!("sha256/{}", "b".repeat(64)),
        },
        staging_key: format!("staging/{attachment_id}"),
        receipt_id: ReceiptId(format!("receipt-{attachment_id}")),
        idempotency_key: attachment_id.into(),
        host_epoch: EPOCH,
        state: AttachmentState::Uploading,
        received_bytes: 0,
    };
    ledger.claim_attachment(&uploading).unwrap();
    let uploaded = AttachmentTransfer {
        received_bytes: 5,
        ..uploading.clone()
    };
    ledger.replace_attachment(&uploading, &uploaded).unwrap();
    let available = AttachmentTransfer {
        state: AttachmentState::Available,
        ..uploaded.clone()
    };
    ledger.replace_attachment(&uploaded, &available).unwrap();
}

pub fn prompt(
    ledger: &SqliteLedger,
    request: &str,
    disposition: AgentChatPromptDisposition,
    attachment_ids: &[&str],
) -> AgentChatPromptSaved {
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(request.into()),
            receipt_id: ReceiptId(format!("receipt-{request}")),
            host_epoch: EPOCH,
            conversation_id: conversation(),
            disposition,
            text: format!("prompt {request}"),
            attachment_ids: attachment_ids.iter().map(|id| (*id).to_owned()).collect(),
            tool_source_ids: vec![],
        })
        .unwrap();
    ledger
        .release_agent_chat_prompt_after_readiness(&saved.message.message_id, &saved.run_id, EPOCH)
        .unwrap();
    saved
}

pub fn running(ledger: &SqliteLedger) -> AgentChatPromptSaved {
    let send = prompt(ledger, "send", AgentChatPromptDisposition::Send, &[]);
    let claimed = ledger
        .claim_agent_chat_prompt_dispatch(DAEMON, EPOCH, provider_of(ledger))
        .unwrap()
        .unwrap();
    assert_eq!(claimed.message.message_id, send.message.message_id);
    ledger
        .begin_agent_chat_prompt_launch(&send.message.message_id, DAEMON, EPOCH)
        .unwrap();
    ledger
        .confirm_agent_chat_prompt_started(&send.message.message_id, DAEMON, EPOCH)
        .unwrap();
    send
}

pub fn join(ledger: &SqliteLedger, turn: &AgentChatPromptSaved) -> Option<String> {
    ledger
        .claim_steered_agent_chat_prompt_for_turn(
            DAEMON,
            EPOCH,
            &turn.run_id,
            &turn.message.turn_id,
        )
        .unwrap()
        .map(|prompt| prompt.message.message_id)
}

pub fn stop(ledger: &SqliteLedger, request: &str) -> gent_types::Receipt {
    ledger
        .stop_agent_chat_conversation(
            &Command {
                receipt_id: ReceiptId(format!("stop-{request}")),
                idempotency_key: format!("agent-chat-interrupt:{request}"),
                host_epoch: EPOCH,
                kind: "agentChatInterrupt".into(),
                payload: json!({"conversationId": "conversation-1", "runId": "run-1"}),
            },
            &conversation(),
        )
        .unwrap()
}

pub fn facts(ledger: &SqliteLedger, kind: &str) -> Vec<Value> {
    ledger
        .read_conversation_activity_page("conversation-1", "run-1", 0, 50)
        .unwrap()
        .facts
        .iter()
        .map(|fact| serde_json::to_value(fact).unwrap())
        .filter(|fact| fact["type"] == kind)
        .collect()
}

pub fn withdrawn(ledger: &SqliteLedger) -> Vec<(String, String, Value)> {
    facts(ledger, "promptWithdrawn")
        .into_iter()
        .map(|fact| {
            (
                fact["messageId"].as_str().unwrap().to_owned(),
                fact["text"].as_str().unwrap().to_owned(),
                fact["attachments"].clone(),
            )
        })
        .collect()
}

pub fn end_turn(ledger: &SqliteLedger, turn: &AgentChatPromptSaved, phase: DurableTurnPhase) {
    ledger
        .settle_agent_chat_prompt_terminal(&turn.message.message_id, DAEMON, EPOCH, phase)
        .unwrap();
}

fn provider_of(ledger: &SqliteLedger) -> AgentChatProvider {
    use gent_ports::AgentChatReadLedger;
    ledger
        .read_agent_chat_detail("conversation-1")
        .unwrap()
        .summary
        .selection
        .provider
}
