use gent_ports::{
    AgentChatProjectionLedger, AgentChatPromptLedger, AgentChatWorkspaceLedger, AttachmentLedger,
    TranscriptLedger, TurnFollowReader,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatProvider,
    AgentChatRequestId, AgentChatRunId, AgentChatSelection, AttachmentMetadata,
    AttachmentReference, AttachmentState, AttachmentTransfer, HostEpoch, ReceiptId,
    WorkspaceRecord,
};
use serde_json::{Value, json};

const CONVERSATION: &str = "conversation-1";

fn conversation() -> AgentChatConversationId {
    AgentChatConversationId(CONVERSATION.into())
}

fn ledger() -> SqliteLedger {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("receipt-conversation".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation(),
                run_id: AgentChatRunId("run-1".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
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

fn stage(ledger: &SqliteLedger, attachment_id: &str, display_name: &str, byte_len: u64) {
    let uploading = AttachmentTransfer {
        metadata: AttachmentMetadata {
            attachment_id: attachment_id.into(),
            display_name: display_name.into(),
            media_type: "image/png".into(),
            byte_len,
            digest_sha256: "a".repeat(64),
            storage_key: format!("sha256/{}", "a".repeat(64)),
        },
        staging_key: format!("staging/{attachment_id}"),
        receipt_id: ReceiptId(format!("receipt-{attachment_id}")),
        idempotency_key: attachment_id.into(),
        host_epoch: HostEpoch(1),
        state: AttachmentState::Uploading,
        received_bytes: 0,
    };
    ledger.claim_attachment(&uploading).unwrap();
    let uploaded = AttachmentTransfer {
        received_bytes: byte_len,
        ..uploading.clone()
    };
    ledger.replace_attachment(&uploading, &uploaded).unwrap();
    let available = AttachmentTransfer {
        state: AttachmentState::Available,
        ..uploaded.clone()
    };
    ledger.replace_attachment(&uploaded, &available).unwrap();
}

fn queue(ledger: &SqliteLedger, request: &str, attachment_ids: &[&str]) -> AgentChatPromptSaved {
    ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(request.into()),
            receipt_id: ReceiptId(format!("receipt-{request}")),
            host_epoch: HostEpoch(1),
            conversation_id: conversation(),
            disposition: AgentChatPromptDisposition::Queue,
            text: format!("prompt {request}"),
            attachment_ids: attachment_ids.iter().map(|id| (*id).to_owned()).collect(),
            tool_source_ids: vec![],
        })
        .unwrap()
}

fn reference(attachment_id: &str, display_name: &str, byte_len: u64) -> AttachmentReference {
    AttachmentReference {
        attachment_id: attachment_id.into(),
        display_name: display_name.into(),
        media_type: "image/png".into(),
        byte_len,
    }
}

fn transcript_attachments(events: &[Value]) -> Vec<Option<Value>> {
    events
        .iter()
        .map(|payload| payload.get("attachments").cloned())
        .collect()
}

#[test]
fn user_messages_carry_their_attachment_references_on_every_read() {
    let ledger = ledger();
    stage(&ledger, "attachment-b", "second.png", 7);
    stage(&ledger, "attachment-a", "first.png", 3);
    let attached = queue(&ledger, "attached", &["attachment-b", "attachment-a"]);
    let plain = queue(&ledger, "plain", &[]);
    let expected = vec![
        reference("attachment-b", "second.png", 7),
        reference("attachment-a", "first.png", 3),
    ];
    let expected_json = json!([
        {"attachmentId": "attachment-b", "displayName": "second.png", "mediaType": "image/png", "byteLen": 7},
        {"attachmentId": "attachment-a", "displayName": "first.png", "mediaType": "image/png", "byteLen": 3},
    ]);

    let page = ledger
        .normalized_transcript_page(&conversation(), 0, 100)
        .unwrap();
    assert_eq!(
        page.events
            .iter()
            .map(|event| event.attachments.clone())
            .collect::<Vec<_>>(),
        vec![expected.clone(), Vec::new()]
    );
    assert_eq!(
        serde_json::to_value(&page.events[0]).unwrap()["attachments"],
        expected_json
    );
    assert!(
        serde_json::to_value(&page.events[1])
            .unwrap()
            .get("attachments")
            .is_none()
    );

    let projection = ledger
        .agent_chat_projection_page(&conversation(), 0, 100)
        .unwrap();
    let projected = projection
        .events
        .iter()
        .filter(|event| event.kind == "transcript")
        .map(|event| event.payload.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        transcript_attachments(&projected),
        vec![Some(expected_json.clone()), None]
    );

    let tail = ledger
        .agent_chat_projection_tail(&conversation(), 100, 100)
        .unwrap();
    let tailed = tail
        .transcript
        .iter()
        .map(|event| event.payload.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        transcript_attachments(&tailed),
        vec![Some(expected_json), None]
    );

    for (saved, attachments) in [(&attached, expected), (&plain, Vec::new())] {
        let followed = ledger
            .turn_follow_page(
                CONVERSATION,
                &saved.run_id.0,
                &saved.message.turn_id,
                0,
                100,
            )
            .unwrap();
        assert_eq!(followed.events.len(), 1);
        assert_eq!(followed.events[0].attachments, attachments);
    }
}
