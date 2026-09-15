use gent_ports::AgentChatPromptLedger;
use gent_protocol::{AgentChatProjectionDelta, AgentChatProjectionFrame, ProjectionCursor};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatPromptOrigin, AgentChatRequestId,
    ReceiptId,
};

use super::{approved, seed_conversation};
use crate::api::RuntimeApi;

#[test]
fn owner_snapshot_and_follow_stream_carry_the_same_typed_prompt_origins() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = approved(directory.path());
    let conversation_id = seed_conversation(&runtime);
    let ledger = SqliteLedger::open(directory.path().join("gent.db")).unwrap();
    let epoch = runtime.status().unwrap().host_epoch;
    let create = |request: &str| AgentChatPromptCreate {
        request_id: AgentChatRequestId(request.into()),
        receipt_id: ReceiptId(request.into()),
        host_epoch: epoch,
        conversation_id: conversation_id.clone(),
        disposition: AgentChatPromptDisposition::Send,
        text: "Keep going".into(),
        attachment_ids: vec![],
        tool_source_ids: vec![],
    };
    let continuation = AgentChatPromptOrigin::GoalContinuation {
        goal_id: "goal-1".into(),
        continues_after_ordinal: 1,
    };
    ledger
        .save_agent_chat_prompt(&create("user-prompt"))
        .unwrap();
    ledger
        .save_agent_chat_prompt_with_origin(&create("continuation-prompt"), &continuation)
        .unwrap();
    let expected = vec![Some(AgentChatPromptOrigin::User), Some(continuation)];
    let AgentChatProjectionFrame::ConversationSnapshot { snapshot, .. } = runtime
        .agent_chat_projection(AgentChatProjectionFrame::ConversationSnapshotRequest {
            request_id: "snapshot".into(),
            conversation_id: conversation_id.0.clone(),
            transcript_limit: 10,
            activity_limit: 10,
        })
        .unwrap()
    else {
        panic!("expected a projection snapshot");
    };
    let restored = snapshot
        .transcript
        .iter()
        .map(|event| event.origin.clone())
        .collect::<Vec<_>>();
    let live = runtime
        .agent_chat_projection_follow(&conversation_id.0, &ProjectionCursor { value: 0 })
        .unwrap()
        .into_iter()
        .filter_map(|delta| match delta {
            AgentChatProjectionDelta::Transcript { event, .. } => Some(event.origin),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(restored, expected);
    assert_eq!(live, expected);
    let wire = serde_json::to_value(&snapshot.transcript[1]).unwrap();
    assert_eq!(
        wire["origin"],
        serde_json::json!({"kind": "goalContinuation", "goalId": "goal-1", "continuesAfterOrdinal": 1})
    );
}
