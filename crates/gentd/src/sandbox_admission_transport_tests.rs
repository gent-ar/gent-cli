use gent_protocol::conversation_links::CONVERSATION_LINKS_CAPABILITY;
use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AGENT_CHAT_PROJECTION_CAPABILITY,
    AGENT_CHAT_PROVIDER_WARM_CAPABILITY, AGENT_CHAT_STEER_ENQUEUE_CAPABILITY,
    AGENT_CHAT_STEER_TURN_CAPABILITY, AgentChatIntentFrame, AgentChatProjectionFrame,
    AgentChatProviderWarm, AgentChatSteerPrompt, Hello, WireFrame, read_frame, read_json_frame,
    write_frame, write_json_frame,
};
use gent_runtime::catalog::{RuntimeCapabilityFeature, RuntimeCapabilityProfile};
use gent_types::{
    AgentChatConversationConfigRecord, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRequestId, AgentChatRunId, AgentChatSelection, CapabilitySet,
    Containment, PROTOCOL_MAX, PROTOCOL_MIN, ReceiptId,
};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{DuplexStream, duplex};

use crate::{CompatibilityAssessment, RuntimeFacade, build_runtime};

const PROVIDER: AgentChatProvider = AgentChatProvider::Codex;

async fn connect(runtime: &RuntimeFacade) -> DuplexStream {
    let (mut client, server) = duplex(1024 * 1024);
    tokio::spawn(crate::transport::serve_connection(server, runtime.clone()));
    write_frame(
        &mut client,
        &WireFrame::Hello(Hello {
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: CapabilitySet(
                [
                    AGENT_CHAT_INTENTS_CAPABILITY,
                    AGENT_CHAT_PROJECTION_CAPABILITY,
                    AGENT_CHAT_STEER_ENQUEUE_CAPABILITY,
                    AGENT_CHAT_STEER_TURN_CAPABILITY,
                    CONVERSATION_LINKS_CAPABILITY,
                    AGENT_CHAT_PROVIDER_WARM_CAPABILITY,
                ]
                .map(str::to_owned)
                .to_vec(),
            ),
        }),
    )
    .await
    .unwrap();
    let WireFrame::Negotiated(negotiated) = read_frame(&mut client).await.unwrap() else {
        panic!("the daemon must negotiate");
    };
    assert_eq!(negotiated.capabilities.0.len(), 6);
    client
}

async fn exchange(client: &mut DuplexStream, request: &impl Serialize) -> Value {
    write_json_frame(client, request).await.unwrap();
    read_json_frame(client).await.unwrap()
}

async fn create(client: &mut DuplexStream, folder: &str, key: &str) -> (String, String) {
    let created = exchange(
        client,
        &AgentChatIntentFrame::CreateConversation {
            request_id: AgentChatRequestId(key.into()),
            receipt_id: ReceiptId(format!("create:{key}")),
            workspace_path: folder.into(),
            selection: Some(AgentChatSelection {
                provider: PROVIDER,
                model: "gpt-5.4-mini".into(),
                effort: AgentChatEffort::Low,
                mode: AgentChatMode::Agent,
            }),
        },
    )
    .await;
    (
        created["body"]["conversationId"].as_str().unwrap().into(),
        created["body"]["runId"].as_str().unwrap().into(),
    )
}

fn ledger(data: &std::path::Path) -> gent_store::SqliteLedger {
    gent_store::SqliteLedger::open(data.join("gent.db")).unwrap()
}

fn contain(data: &std::path::Path, conversation_id: &str) {
    gent_ports::AgentChatConversationConfigLedger::save_conversation_config(
        &ledger(data),
        &AgentChatConversationConfigRecord {
            conversation_id: AgentChatConversationId(conversation_id.into()),
            revision: 1,
            system_prompt: None,
            append_system_prompt: false,
            max_turns: None,
            thinking_enabled: None,
            disallowed_tools: Vec::new(),
            containment: Containment::Sandboxed,
        },
    )
    .unwrap();
}

fn admissions(conversation_id: &str, run_id: &str) -> Vec<(String, AgentChatIntentFrame)> {
    let conversation = || AgentChatConversationId(conversation_id.into());
    let request = |key: &str| AgentChatRequestId(format!("{conversation_id}:{key}"));
    let receipt = |key: &str| ReceiptId(format!("{conversation_id}:{key}"));
    vec![
        AgentChatIntentFrame::SendPrompt {
            request_id: request("send"),
            receipt_id: receipt("send"),
            conversation_id: conversation(),
            text: "hello".into(),
            attachment_ids: Vec::new(),
        },
        AgentChatIntentFrame::QueuePrompt {
            request_id: request("queue"),
            receipt_id: receipt("queue"),
            conversation_id: conversation(),
            text: "later".into(),
            attachment_ids: Vec::new(),
        },
        AgentChatIntentFrame::SendPromptWithTools {
            request_id: request("send-tools"),
            receipt_id: receipt("send-tools"),
            conversation_id: conversation(),
            text: "hello".into(),
            attachment_ids: Vec::new(),
            tool_source_ids: Vec::new(),
        },
        AgentChatIntentFrame::QueuePromptWithTools {
            request_id: request("queue-tools"),
            receipt_id: receipt("queue-tools"),
            conversation_id: conversation(),
            text: "later".into(),
            attachment_ids: Vec::new(),
            tool_source_ids: Vec::new(),
        },
        AgentChatIntentFrame::SteerQueuedPrompt {
            request_id: request("steer-queued"),
            receipt_id: receipt("steer-queued"),
            conversation_id: conversation(),
            message_id: "message".into(),
        },
        AgentChatIntentFrame::SteerPromptForTurn(AgentChatSteerPrompt {
            request_id: request("steer"),
            receipt_id: receipt("steer"),
            conversation_id: conversation(),
            expected_run_id: AgentChatRunId(run_id.into()),
            expected_turn_id: "turn".into(),
            text: "now".into(),
            attachment_ids: Vec::new(),
            tool_source_ids: Vec::new(),
        }),
        AgentChatIntentFrame::SteerQueuedPromptForTurn {
            request_id: request("steer-queued-turn"),
            receipt_id: receipt("steer-queued-turn"),
            conversation_id: conversation(),
            message_id: "message".into(),
            expected_run_id: AgentChatRunId(run_id.into()),
            expected_turn_id: "turn".into(),
        },
        AgentChatIntentFrame::ContinueFromSavedHistory {
            request_id: request("continue"),
            receipt_id: receipt("continue"),
            conversation_id: conversation(),
            message_id: "message".into(),
        },
    ]
    .into_iter()
    .map(|frame| {
        let receipt_id = serde_json::to_value(&frame).unwrap()["body"]["receiptId"]
            .as_str()
            .unwrap()
            .to_owned();
        (receipt_id, frame)
    })
    .collect()
}

fn lookup(data: &std::path::Path, receipt_id: &str) -> Value {
    serde_json::to_value(
        ledger(data)
            .lookup_receipt(&ReceiptId(receipt_id.into()))
            .unwrap(),
    )
    .unwrap()["state"]
        .clone()
}

async fn snapshot(client: &mut DuplexStream, conversation_id: &str) -> Value {
    exchange(
        client,
        &AgentChatProjectionFrame::ConversationSnapshotRequest {
            request_id: "snapshot".into(),
            conversation_id: conversation_id.into(),
            transcript_limit: 50,
            activity_limit: 50,
        },
    )
    .await
}

#[tokio::test]
async fn a_sandbox_the_provider_cannot_enforce_is_refused_before_any_receipt() {
    let data = tempfile::tempdir().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let folder = folder.path().display().to_string();
    let runtime = build_runtime(
        data.path(),
        &RuntimeCapabilityProfile::new([
            RuntimeCapabilityFeature::AgentChat,
            RuntimeCapabilityFeature::AgentChatProjection,
        ]),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let mut client = connect(&runtime).await;
    let (contained, contained_run) = create(&mut client, &folder, "contained").await;
    let (open, open_run) = create(&mut client, &folder, "open").await;
    contain(data.path(), &contained);
    for (receipt_id, frame) in admissions(&open, &open_run).into_iter().take(2) {
        let reply = exchange(&mut client, &frame).await;
        assert_eq!(reply["type"], "accepted", "{reply}");
        assert_eq!(lookup(data.path(), &receipt_id), "accepted");
    }
    let Err(reason) = crate::permission_backend_registry::require(PROVIDER, Containment::Sandboxed)
    else {
        return;
    };
    let refused = json!({"type": "error", "body": {
        "code": "sandboxUnavailable",
        "message": reason,
        "sandbox": {"provider": "codex", "containment": "sandboxed"}
    }});
    let before = snapshot(&mut client, &contained).await;
    assert_eq!(before["type"], "conversationSnapshot");
    for (receipt_id, frame) in admissions(&contained, &contained_run) {
        assert_eq!(exchange(&mut client, &frame).await, refused, "{receipt_id}");
        assert_eq!(lookup(data.path(), &receipt_id), "neverSeen");
    }
    let delivered = AgentChatIntentFrame::SendToConversation {
        request_id: AgentChatRequestId("deliver".into()),
        receipt_id: ReceiptId("deliver".into()),
        from_conversation_id: AgentChatConversationId(open.clone()),
        target_conversation_id: AgentChatConversationId(contained.clone()),
        message: "over to you".into(),
    };
    assert_eq!(exchange(&mut client, &delivered).await, refused);
    assert_eq!(lookup(data.path(), "deliver"), "neverSeen");
    let warm = AgentChatIntentFrame::WarmProvider(AgentChatProviderWarm {
        request_id: AgentChatRequestId("warm".into()),
        conversation_id: AgentChatConversationId(contained.clone()),
        run_id: AgentChatRunId(contained_run.clone()),
    });
    assert_eq!(exchange(&mut client, &warm).await, refused);
    let linked = AgentChatIntentFrame::CreateLinkedConversation {
        request_id: AgentChatRequestId("linked".into()),
        receipt_id: ReceiptId("linked".into()),
        parent_conversation_id: AgentChatConversationId(contained.clone()),
        workspace_path: None,
        selection: None,
        label: "child".into(),
        prompt: Some("start".into()),
        origin_tool_use_id: None,
    };
    assert_eq!(exchange(&mut client, &linked).await, refused);
    assert_eq!(lookup(data.path(), "linked"), "neverSeen");
    assert_eq!(snapshot(&mut client, &contained).await, before);
}
