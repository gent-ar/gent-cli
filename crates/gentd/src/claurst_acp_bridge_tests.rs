use std::collections::VecDeque;

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
        gent_testkit::host_absolute_path("/workspace"),
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
        gent_testkit::host_absolute_path("/workspace"),
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
        gent_testkit::host_absolute_path("/workspace"),
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

struct Recording {
    writes: std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
    reads: VecDeque<Vec<u8>>,
}
impl ClaurstAcpStdio for Recording {
    fn write_frame(&mut self, frame: &[u8]) -> Result<(), String> {
        self.writes.lock().unwrap().push(frame.to_vec());
        Ok(())
    }
    fn try_read_frame(&mut self, _: usize) -> Result<Option<Vec<u8>>, String> {
        Ok(self.reads.pop_front())
    }
}

#[tokio::test]
async fn a_user_prompt_carries_the_gent_resolved_active_goal_like_claude_and_codex() {
    let goal = gent_types::GoalRecord {
        schema_version: gent_types::GOAL_SCHEMA_VERSION,
        binding: gent_types::GoalBinding {
            goal_id: "goal-1".into(),
            conversation_id: AgentChatConversationId("c-1".into()),
        },
        revision: 3,
        status: gent_types::GoalStatus::Active,
        reason: gent_types::GoalStatusReason::UserSet,
        objective: "Ship the steer".into(),
        note: None,
        time_used_seconds: 0,
        active_since: Some(1),
        tokens_used: 0,
        token_budget: None,
        turns_without_progress: 0,
        accounted_through_ordinal: 0,
        created_at: 1,
        updated_at: 1,
    };
    let writes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let bridge = ClaurstAcpBridge::new(
        gent_testkit::host_absolute_path("/workspace"),
        Recording {
            writes: std::sync::Arc::clone(&writes),
            reads: VecDeque::from([
                frame(serde_json::json!({"id": 1, "result": {}})),
                frame(serde_json::json!({"id": 2, "result": {"sessionId": "acp-1"}})),
            ]),
        },
        vec![],
    );
    let mut start = request();
    start.goal = Some(gent_ports::ClaurstGoalProjection {
        run_id: "run-1".into(),
        source_id: ClaurstSourceId("source-1".into()),
        goal: gent_types::GoalProjection::from_active(&goal).unwrap(),
    });
    bridge.start(start).await.unwrap();

    let prompt = writes
        .lock()
        .unwrap()
        .iter()
        .map(|frame| serde_json::from_slice::<serde_json::Value>(frame).unwrap())
        .find(|frame| frame["method"] == "session/prompt")
        .unwrap();
    let text = prompt["params"]["prompt"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("Gent-owned active goal context"));
    assert!(text.contains("\"goalId\":\"goal-1\"") && text.contains("Ship the steer"));
    assert!(text.ends_with("User prompt:\nhello"));
}

#[tokio::test]
async fn a_start_renders_history_within_the_local_model_budget() {
    let context = {
        use sha2::{Digest, Sha256};
        let text = "remember CODE-1".to_owned();
        let entry = gent_types::ConversationContentEntry {
            message_id: "m-1".into(),
            turn_id: "t-1".into(),
            run_id: "run-0".into(),
            ordinal: 1,
            text_digest_sha256: format!("{:x}", Sha256::digest(text.as_bytes())),
            text,
        };
        let event = gent_types::NormalizedTranscriptEvent {
            cursor: 1,
            event_id: "e-1".into(),
            turn_id: "t-1".into(),
            run_id: "run-0".into(),
            kind: gent_types::NormalizedTranscriptKind::ToolActivity,
            text: format!("HEAD{}TAIL", "x".repeat(40_000)),
            is_partial: false,
            origin: None,
            attachments: Vec::new(),
        };
        let mut digest = Sha256::new();
        digest.update(1_u64.to_be_bytes());
        digest.update(entry.text_digest_sha256.as_bytes());
        digest.update([0]);
        FrozenConversationContext {
            conversation_id: AgentChatConversationId("c-1".into()),
            context_through_ordinal: 1,
            transcript_digest_sha256: FrozenConversationContext::transcript_digest(
                std::slice::from_ref(&event),
            ),
            transcript_events: vec![event],
            entries: vec![entry],
            content_digest_sha256: format!("{:x}", digest.finalize()),
            summary: None,
            earlier_history_omitted: false,
        }
    };
    let bridge = ClaurstAcpBridge::new(
        gent_testkit::host_absolute_path("/workspace"),
        Fake {
            writes: vec![],
            reads: VecDeque::from([
                frame(serde_json::json!({"id": 1, "result": {}})),
                frame(serde_json::json!({"id": 2, "result": {"sessionId": "acp-1"}})),
            ]),
        },
        vec![],
    )
    .with_history_input_bytes(12_000);
    let mut request = request();
    request.context = context;
    bridge.start(request).await.unwrap();
    let state = bridge.state.lock().unwrap();
    assert_eq!(state.history_input_bytes, 12_000);
}

struct SharedFake {
    writes: std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
    reads: VecDeque<Vec<u8>>,
}
impl ClaurstAcpStdio for SharedFake {
    fn write_frame(&mut self, frame: &[u8]) -> Result<(), String> {
        self.writes.lock().unwrap().push(frame.to_vec());
        Ok(())
    }
    fn try_read_frame(&mut self, _: usize) -> Result<Option<Vec<u8>>, String> {
        Ok(self.reads.pop_front())
    }
}

#[tokio::test]
async fn later_turns_of_one_run_continue_its_claurst_session_until_a_failure() {
    let turn = |source: &str| {
        let mut request = request();
        request.source_id = ClaurstSourceId(source.into());
        request.turn_id = format!("turn-{source}");
        request.prompt = format!("prompt {source}");
        request
    };
    let drain = |source: &str| ClaurstDrainRequest {
        run_id: "run-1".into(),
        source_id: ClaurstSourceId(source.into()),
        after_cursor: 0,
        limit: 64,
    };
    let writes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let bridge = ClaurstAcpBridge::new(
        gent_testkit::host_absolute_path("/workspace"),
        SharedFake {
            writes: std::sync::Arc::clone(&writes),
            reads: VecDeque::from([
                frame(serde_json::json!({"id": 1, "result": {}})),
                frame(serde_json::json!({"id": 2, "result": {"sessionId": "acp-1"}})),
                frame(
                    serde_json::json!({"method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"text":"one"}}}}),
                ),
                frame(serde_json::json!({"id": 3, "result": {"stopReason": "end_turn"}})),
                frame(serde_json::json!({"id": 4, "result": {"stopReason": "max_tokens"}})),
                frame(serde_json::json!({"id": 5, "result": {"sessionId": "acp-2"}})),
            ]),
        },
        vec![],
    );
    assert_eq!(
        bridge.start(turn("a")).await.unwrap().opaque_session_id,
        "acp-1"
    );
    assert!(bridge.drain(drain("a")).await.unwrap().terminal.is_some());
    assert_eq!(
        bridge.start(turn("b")).await.unwrap().opaque_session_id,
        "acp-1"
    );
    assert!(matches!(
        bridge.drain(drain("b")).await.unwrap().terminal,
        Some(gent_ports::ClaurstTerminal::Failed { .. })
    ));
    assert_eq!(
        bridge.start(turn("c")).await.unwrap().opaque_session_id,
        "acp-2"
    );
    let writes = writes
        .lock()
        .unwrap()
        .iter()
        .map(|frame| serde_json::from_slice::<serde_json::Value>(frame).unwrap())
        .collect::<Vec<_>>();
    let methods = writes
        .iter()
        .map(|frame| frame["method"].as_str().unwrap_or_default().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        methods,
        [
            "initialize",
            "session/new",
            "session/prompt",
            "session/prompt",
            "session/new",
            "session/prompt"
        ]
    );
    assert_eq!(writes[3]["params"]["prompt"][0]["text"], "prompt b");
    assert_eq!(writes[3]["params"]["sessionId"], "acp-1");
}

fn summarized(omitted: bool) -> FrozenConversationContext {
    use sha2::{Digest, Sha256};
    FrozenConversationContext {
        conversation_id: AgentChatConversationId("c-1".into()),
        context_through_ordinal: 1,
        entries: vec![],
        transcript_events: vec![],
        transcript_digest_sha256: FrozenConversationContext::transcript_digest(&[]),
        content_digest_sha256: format!("{:x}", Sha256::digest(b"")),
        summary: Some(gent_types::ConversationContextSummary {
            covers_through_ordinal: 1,
            imports_covered: false,
            text: "The user planted LARK-7.".into(),
        }),
        earlier_history_omitted: omitted,
    }
}

#[tokio::test]
async fn a_new_summary_or_a_truncated_history_starts_a_fresh_seeded_session() {
    let turn = |source: &str, context: FrozenConversationContext| {
        let mut request = request();
        request.source_id = ClaurstSourceId(source.into());
        request.turn_id = format!("turn-{source}");
        request.prompt = format!("prompt {source}");
        request.context = context;
        request
    };
    let drain = |source: &str| ClaurstDrainRequest {
        run_id: "run-1".into(),
        source_id: ClaurstSourceId(source.into()),
        after_cursor: 0,
        limit: 64,
    };
    let reply = || {
        frame(
            serde_json::json!({"method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"text":"ok"}}}}),
        )
    };
    let end = |id: u64| frame(serde_json::json!({"id": id, "result": {"stopReason": "end_turn"}}));
    let session =
        |id: u64, name: &str| frame(serde_json::json!({"id": id, "result": {"sessionId": name}}));
    let writes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let bridge = ClaurstAcpBridge::new(
        gent_testkit::host_absolute_path("/workspace"),
        SharedFake {
            writes: std::sync::Arc::clone(&writes),
            reads: VecDeque::from([
                frame(serde_json::json!({"id": 1, "result": {}})),
                session(2, "acp-1"),
                reply(),
                end(3),
                session(4, "acp-2"),
                reply(),
                end(5),
                reply(),
                end(6),
                session(7, "acp-3"),
                reply(),
                end(8),
            ]),
        },
        vec![],
    );
    let mut sessions = Vec::new();
    for (source, context) in [
        (
            "a",
            FrozenConversationContext::cleared(AgentChatConversationId("c-1".into())),
        ),
        ("b", summarized(false)),
        ("c", summarized(false)),
        ("d", summarized(true)),
    ] {
        sessions.push(
            bridge
                .start(turn(source, context))
                .await
                .unwrap()
                .opaque_session_id,
        );
        assert!(
            bridge
                .drain(drain(source))
                .await
                .unwrap()
                .terminal
                .is_some()
        );
    }
    assert_eq!(sessions, ["acp-1", "acp-2", "acp-2", "acp-3"]);
    let prompts = writes
        .lock()
        .unwrap()
        .iter()
        .map(|frame| serde_json::from_slice::<serde_json::Value>(frame).unwrap())
        .filter(|frame| frame["method"] == "session/prompt")
        .map(|frame| {
            frame["params"]["prompt"][0]["text"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(prompts[0], "prompt a");
    assert!(prompts[1].contains("The user planted LARK-7.") && prompts[1].ends_with("prompt b"));
    assert_eq!(prompts[2], "prompt c");
    assert!(prompts[3].contains("Earlier history was omitted") && prompts[3].ends_with("prompt d"));
}
