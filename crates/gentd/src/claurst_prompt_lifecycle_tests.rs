use gent_ports::agent_chat_terminal_settlement::AgentChatTerminalSettlementReader;
use gent_ports::{
    AgentChatConversationConfigLedger, AgentChatPromptLedger, AgentChatWorkspaceLedger,
    AttachmentLedger, ClaurstCheckpoint, ClaurstDrainBatch, ClaurstSessionBinding, ClaurstSourceId,
    ConversationLedger, Ledger, RunLease,
};
use gent_store::SqliteLedger;
use gent_testkit::FakePrivateClaurstBridge;
use gent_types::{
    AgentChatConversationConfigRecord, AgentChatConversationCreate, AgentChatConversationId,
    AgentChatEffort, AgentChatMode, AgentChatPromptCreate, AgentChatPromptDisposition,
    AgentChatProvider, AgentChatRequestId, AgentChatRunId, AgentChatSelection, AttachmentMetadata,
    AttachmentState, AttachmentTransfer, DurableTurnPhase, HostEpoch, ReceiptId, WorkspaceRecord,
};
use sha2::{Digest, Sha256};

use super::{AsyncOrdinaryLifecycleHost, ClaurstPromptLifecycle};

#[tokio::test]
async fn empty_claurst_outbox_is_idle_after_recovery() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let mut lifecycle = ClaurstPromptLifecycle::new(
        ledger,
        FakePrivateClaurstBridge::default(),
        "gentd-1".into(),
        HostEpoch(1),
    );
    lifecycle.activate_recovery().await.unwrap();
    assert!(!lifecycle.drive_once().await.unwrap());
}

#[tokio::test]
async fn claimed_claurst_prompt_acquires_the_run_lease_before_recording_its_session() {
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
                    provider: AgentChatProvider::Claurst,
                    model: "qwen2-5-coder-7b-instruct-q4-k-m".into(),
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
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-request".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            disposition: AgentChatPromptDisposition::Send,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: "hello".into(),
        })
        .unwrap();
    crate::readiness_test_support::release(&ledger, &saved);
    let material = format!(
        "{}\0{}\0{}",
        saved.run_id.0, saved.message.turn_id, saved.message.message_id
    );
    let binding = gent_ports::ClaurstSessionBinding {
        run_id: "run-a".into(),
        source_id: gent_ports::ClaurstSourceId(format!(
            "gent-{}",
            hex::encode(Sha256::digest(material.as_bytes()))
        )),
        opaque_session_id: "acp-session".into(),
    };
    let bridge = FakePrivateClaurstBridge::default();
    bridge.push_start_binding(binding);
    let mut lifecycle =
        ClaurstPromptLifecycle::new(ledger.clone(), bridge, "gentd-1".into(), HostEpoch(1));
    lifecycle.activate_recovery().await.unwrap();
    lifecycle.drive_once().await.unwrap();
    assert_eq!(
        ledger.find_run_lease("run-a").unwrap(),
        Some(RunLease {
            run_id: "run-a".into(),
            coordinator_id: "gentd-1".into(),
            host_epoch: HostEpoch(1)
        })
    );
    assert_eq!(
        ledger
            .find_run_session_binding("run-a")
            .unwrap()
            .unwrap()
            .provider_session_id,
        "acp-session"
    );
}

#[tokio::test]
async fn appended_conversation_config_prefixes_the_prompt_sent_to_claurst() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &conversation(),
            &WorkspaceRecord {
                workspace_id: "workspace-a".into(),
                canonical_path: "/workspace-a".into(),
            },
        )
        .unwrap();
    ledger
        .save_conversation_config(&AgentChatConversationConfigRecord {
            conversation_id: AgentChatConversationId("conversation-a".into()),
            revision: 1,
            system_prompt: Some("Prefer terse replies.".into()),
            append_system_prompt: true,
            max_turns: None,
            disallowed_tools: Vec::new(),
        })
        .unwrap();
    let saved = ledger.save_agent_chat_prompt(&prompt()).unwrap();
    crate::readiness_test_support::release(&ledger, &saved);
    let bridge = FakePrivateClaurstBridge::default();
    bridge.push_start_binding(binding(&saved));
    let mut lifecycle = ClaurstPromptLifecycle::new(
        ledger.clone(),
        bridge.clone(),
        "gentd-1".into(),
        HostEpoch(1),
    );
    lifecycle.activate_recovery().await.unwrap();
    lifecycle.drive_once().await.unwrap();
    let starts = bridge.starts();
    assert_eq!(starts.len(), 1);
    assert!(starts[0].prompt.starts_with("Prefer terse replies.\n\n"));
    assert!(starts[0].prompt.ends_with("hello"));
}

#[tokio::test]
async fn terminal_claurst_drain_atomically_settles_its_exact_durable_turn() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &conversation(),
            &WorkspaceRecord {
                workspace_id: "workspace-a".into(),
                canonical_path: "/workspace-a".into(),
            },
        )
        .unwrap();
    let saved = ledger.save_agent_chat_prompt(&prompt()).unwrap();
    crate::readiness_test_support::release(&ledger, &saved);
    let binding = binding(&saved);
    let bridge = FakePrivateClaurstBridge::default();
    bridge.push_start_binding(binding.clone());
    bridge.push_batch(ClaurstDrainBatch {
        facts: vec![],
        permissions: vec![],
        checkpoint: Some(ClaurstCheckpoint {
            run_id: binding.run_id.clone(),
            source_id: binding.source_id.clone(),
            cursor: 0,
            state_digest_sha256: "d".repeat(64),
        }),
        session_binding: Some(binding),
        terminal: Some(gent_ports::ClaurstTerminal::Completed),
    });
    let mut lifecycle =
        ClaurstPromptLifecycle::new(ledger.clone(), bridge, "gentd-1".into(), HostEpoch(1));
    lifecycle.activate_recovery().await.unwrap();
    lifecycle.drive_once().await.unwrap();
    lifecycle.drive_once().await.unwrap();
    assert_eq!(
        ledger
            .find_turn(&saved.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        DurableTurnPhase::Completed
    );
    assert!(
        ledger
            .read_agent_chat_terminal_settlement(&saved.run_id.0, &saved.message.turn_id)
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn unsupported_attachments_fail_before_claurst_acp_receives_a_prompt() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &conversation(),
            &WorkspaceRecord {
                workspace_id: "workspace-a".into(),
                canonical_path: "/workspace-a".into(),
            },
        )
        .unwrap();
    let initial = AttachmentTransfer {
        metadata: AttachmentMetadata {
            attachment_id: "attachment-a".into(),
            display_name: "image.png".into(),
            media_type: "image/png".into(),
            byte_len: 3,
            digest_sha256: "a".repeat(64),
            storage_key: "sha256/attachment-a".into(),
        },
        staging_key: "staging/attachment-a".into(),
        receipt_id: ReceiptId("attachment-receipt".into()),
        idempotency_key: "attachment-key".into(),
        host_epoch: HostEpoch(1),
        state: AttachmentState::Uploading,
        received_bytes: 0,
    };
    ledger.claim_attachment(&initial).unwrap();
    let mut uploaded = initial.clone();
    uploaded.received_bytes = 3;
    ledger.replace_attachment(&initial, &uploaded).unwrap();
    let mut available = uploaded.clone();
    available.state = AttachmentState::Available;
    ledger.replace_attachment(&uploaded, &available).unwrap();
    let mut request = prompt();
    request.attachment_ids = vec!["attachment-a".into()];
    let saved = ledger.save_agent_chat_prompt(&request).unwrap();
    crate::readiness_test_support::release(&ledger, &saved);
    let bridge = FakePrivateClaurstBridge::default();
    let mut lifecycle =
        ClaurstPromptLifecycle::new(ledger, bridge.clone(), "gentd-1".into(), HostEpoch(1));
    lifecycle.activate_recovery().await.unwrap();
    assert!(lifecycle.drive_once().await.unwrap());
    assert!(bridge.starts().is_empty());
}

fn conversation() -> AgentChatConversationCreate {
    AgentChatConversationCreate {
        receipt_id: ReceiptId("conversation-receipt".into()),
        idempotency_key: "conversation-key".into(),
        host_epoch: HostEpoch(1),
        conversation_id: AgentChatConversationId("conversation-a".into()),
        run_id: AgentChatRunId("run-a".into()),
        selection: AgentChatSelection {
            provider: AgentChatProvider::Claurst,
            model: "qwen2-5-coder-7b-instruct-q4-k-m".into(),
            effort: AgentChatEffort::Medium,
            mode: AgentChatMode::Agent,
        },
    }
}

fn prompt() -> AgentChatPromptCreate {
    AgentChatPromptCreate {
        request_id: AgentChatRequestId("prompt-request".into()),
        receipt_id: ReceiptId("prompt-receipt".into()),
        host_epoch: HostEpoch(1),
        conversation_id: AgentChatConversationId("conversation-a".into()),
        disposition: AgentChatPromptDisposition::Send,
        attachment_ids: vec![],
        tool_source_ids: vec![],
        text: "hello".into(),
    }
}

fn binding(saved: &gent_types::AgentChatPromptSaved) -> ClaurstSessionBinding {
    let material = format!(
        "{}\0{}\0{}",
        saved.run_id.0, saved.message.turn_id, saved.message.message_id
    );
    ClaurstSessionBinding {
        run_id: saved.run_id.0.clone(),
        source_id: ClaurstSourceId(format!("gent-{}", hex::encode(Sha256::digest(material)))),
        opaque_session_id: "acp-session".into(),
    }
}
