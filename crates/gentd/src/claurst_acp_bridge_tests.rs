use std::{collections::VecDeque, path::PathBuf};

use gent_ports::{
    ClaurstDrainRequest, ClaurstSourceId, ClaurstStartRequest, ClaurstSubmitRequest,
    PrivateClaurstBridge,
};
use gent_types::{AgentChatConversationId, FrozenConversationContext, NormalizedProviderEvent};

use super::{ClaurstAcpBridge, ClaurstAcpStdio};

struct Fake {
    writes: Vec<Vec<u8>>,
    reads: VecDeque<Vec<u8>>,
}
impl ClaurstAcpStdio for Fake {
    fn write_frame(&mut self, frame: &[u8]) -> Result<(), String> {
        self.writes.push(frame.to_vec());
        Ok(())
    }
    fn try_read_frame(&mut self, _: usize) -> Result<Option<Vec<u8>>, String> {
        Ok(self.reads.pop_front())
    }
}
fn frame(value: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&value).unwrap()
}
fn request() -> ClaurstStartRequest {
    ClaurstStartRequest {
        run_id: "run-1".into(),
        source_id: ClaurstSourceId("source-1".into()),
        turn_id: "turn-1".into(),
        prompt: "hello".into(),
        context: FrozenConversationContext::cleared(AgentChatConversationId("c-1".into())),
        attachments: vec![],
        goal: None,
    }
}

#[tokio::test]
async fn starts_prompts_and_drains_cursor_sealed_normalized_facts() {
    let bridge = ClaurstAcpBridge::new(
        PathBuf::from("/workspace"),
        Fake {
            writes: vec![],
            reads: VecDeque::from([
                frame(serde_json::json!({"id": 1, "result": {}})),
                frame(serde_json::json!({"id": 2, "result": {"sessionId": "acp-1"}})),
                frame(
                    serde_json::json!({"method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"text":"hello back"}}}}),
                ),
                frame(serde_json::json!({"id":3,"result":{"stopReason":"end_turn"}})),
            ]),
        },
        vec![],
    );
    let binding = bridge.start(request()).await.unwrap();
    bridge.bind_session(binding.clone()).await.unwrap();
    let batch = bridge
        .drain(ClaurstDrainRequest {
            run_id: "run-1".into(),
            source_id: ClaurstSourceId("source-1".into()),
            after_cursor: 0,
            limit: 64,
        })
        .await
        .unwrap();
    assert_eq!(batch.facts.len(), 2);
    assert!(
        matches!(batch.facts[0].value, gent_ports::ClaurstFactValue::Event(NormalizedProviderEvent::Output { ref text, is_partial: true }) if text == "hello back")
    );
    assert!(
        matches!(batch.facts[1].value, gent_ports::ClaurstFactValue::Event(NormalizedProviderEvent::Output { ref text, is_partial: false }) if text == "hello back")
    );
    assert_eq!(batch.facts[0].cursor, 1);
    assert_eq!(batch.facts[1].cursor, 2);
    assert_eq!(batch.checkpoint.unwrap().cursor, 2);
    assert_eq!(batch.terminal, Some(gent_ports::ClaurstTerminal::Completed));
    assert!(
        bridge
            .drain(ClaurstDrainRequest {
                run_id: "run-1".into(),
                source_id: ClaurstSourceId("source-1".into()),
                after_cursor: 2,
                limit: 64
            })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn rejects_cross_session_and_overlapping_follow_up_prompts() {
    let bridge = ClaurstAcpBridge::new(
        PathBuf::from("/workspace"),
        Fake {
            writes: vec![],
            reads: VecDeque::from([
                frame(serde_json::json!({"id": 1, "result": {}})),
                frame(serde_json::json!({"id": 2, "result": {"sessionId": "acp-1"}})),
            ]),
        },
        vec![],
    );
    let binding = bridge.start(request()).await.unwrap();
    let mut wrong = binding.clone();
    wrong.opaque_session_id = "wrong".into();
    assert!(
        bridge
            .submit(ClaurstSubmitRequest {
                binding: wrong,
                turn_id: "turn-2".into(),
                prompt: "again".into(),
                attachments: vec![],
                goal: None
            })
            .await
            .is_err()
    );
    assert!(
        bridge
            .submit(ClaurstSubmitRequest {
                binding,
                turn_id: "turn-2".into(),
                prompt: "again".into(),
                attachments: vec![],
                goal: None
            })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancels_only_the_exact_active_binding_without_settling_it() {
    let bridge = ClaurstAcpBridge::new(
        PathBuf::from("/workspace"),
        Fake {
            writes: vec![],
            reads: VecDeque::from([
                frame(serde_json::json!({"id": 1, "result": {}})),
                frame(serde_json::json!({"id": 2, "result": {"sessionId": "acp-1"}})),
                frame(serde_json::json!({"id": 4, "result": {}})),
            ]),
        },
        vec![],
    );
    let binding = bridge.start(request()).await.unwrap();
    let mut wrong = binding.clone();
    wrong.run_id = "run-other".into();
    assert!(bridge.cancel(wrong).await.is_err());
    bridge.cancel(binding.clone()).await.unwrap();
    let batch = bridge
        .drain(ClaurstDrainRequest {
            run_id: binding.run_id,
            source_id: binding.source_id,
            after_cursor: 0,
            limit: 1,
        })
        .await
        .unwrap();
    assert_eq!(batch.terminal, None);
}
