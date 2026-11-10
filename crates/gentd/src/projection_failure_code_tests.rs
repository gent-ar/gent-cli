use gent_ports::{AgentChatPromptLedger, TranscriptLedger};
use gent_protocol::{AgentChatProjectionDelta, AgentChatProjectionFrame, ProjectionCursor};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatFailureCode, AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatRequestId,
    NormalizedTranscriptAppend, NormalizedTranscriptKind, ReceiptId,
};
use serde_json::{Value, json};

use super::{approved, seed_conversation};
use crate::api::RuntimeApi;

#[test]
fn snapshot_and_follow_stream_expose_each_notice_failure_code() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = approved(directory.path());
    let conversation_id = seed_conversation(&runtime);
    let ledger = SqliteLedger::open(directory.path().join("gent.db")).unwrap();
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt".into()),
            receipt_id: ReceiptId("prompt".into()),
            host_epoch: runtime.status().unwrap().host_epoch,
            conversation_id: conversation_id.clone(),
            disposition: AgentChatPromptDisposition::Send,
            text: "Continue".into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    let codes = [
        Some(AgentChatFailureCode::ProviderAuthRequired),
        Some(AgentChatFailureCode::GentdRestarted),
        Some(AgentChatFailureCode::SandboxUnavailable),
        Some(AgentChatFailureCode::SandboxAdmissionFailed),
        None,
    ];
    for (index, code) in codes.iter().enumerate() {
        ledger
            .append_normalized_transcript(
                &conversation_id,
                &NormalizedTranscriptAppend {
                    event_id: format!(
                        "{}notice-{index}",
                        code.map_or("", AgentChatFailureCode::event_prefix)
                    ),
                    turn_id: saved.message.turn_id.clone(),
                    run_id: saved.run_id.0.clone(),
                    kind: NormalizedTranscriptKind::Notice,
                    text: format!("notice {index}"),
                    is_partial: false,
                },
            )
            .unwrap();
    }
    let expected = [
        json!("providerAuthRequired"),
        json!("gentdRestarted"),
        json!("sandboxUnavailable"),
        json!("sandboxAdmissionFailed"),
        Value::Null,
    ];
    let notices = |events: Vec<Value>| {
        events
            .into_iter()
            .filter(|event| event["kind"] == "notice")
            .map(|event| event.get("failureCode").cloned().unwrap_or(Value::Null))
            .collect::<Vec<_>>()
    };
    let AgentChatProjectionFrame::ConversationSnapshot { snapshot, .. } = runtime
        .agent_chat_projection(AgentChatProjectionFrame::ConversationSnapshotRequest {
            request_id: "snapshot".into(),
            conversation_id: conversation_id.0.clone(),
            transcript_limit: 20,
            activity_limit: 20,
        })
        .unwrap()
    else {
        panic!("expected a projection snapshot");
    };
    let snapshot_wire = serde_json::to_value(&snapshot).unwrap()["transcript"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(notices(snapshot_wire), expected);
    let follow_wire = runtime
        .agent_chat_projection_follow(&conversation_id.0, &ProjectionCursor { value: 0 })
        .unwrap()
        .into_iter()
        .filter(|delta| matches!(delta, AgentChatProjectionDelta::Transcript { .. }))
        .map(|delta| serde_json::to_value(&delta).unwrap()["body"]["event"].clone())
        .collect();
    assert_eq!(notices(follow_wire), expected);
}
