use gent_types::{AgentChatConversationId, FrozenConversationContext, RunVersionLock};

use super::{ClaudeRunStart, ClaudeRunnerError, input_frame};

#[test]
fn fresh_claude_context_cannot_resume_a_native_session() {
    let start = ClaudeRunStart {
        fresh_context: Some(FrozenConversationContext::cleared(AgentChatConversationId(
            "conversation".into(),
        ))),
        resume_session_id: Some("native-session".into()),
        ..start()
    };
    assert!(matches!(
        input_frame(&start),
        Err(ClaudeRunnerError::InvalidPrompt)
    ));
}

#[test]
fn cleared_fresh_claude_context_uses_no_native_session_field() {
    let start = ClaudeRunStart {
        fresh_context: Some(FrozenConversationContext::cleared(AgentChatConversationId(
            "conversation".into(),
        ))),
        ..start()
    };
    let value: serde_json::Value = serde_json::from_slice(&input_frame(&start).unwrap()).unwrap();
    assert!(value.get("session_id").is_none());
    assert!(
        value["message"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("prompt")
    );
}

fn start() -> ClaudeRunStart {
    ClaudeRunStart {
        run_id: "run".into(),
        lock: RunVersionLock {
            provider: "claude".into(),
            canonical_path: "/locked/claude".into(),
            file_identity: "1:2".into(),
            digest_sha256: "a".repeat(64),
            version: "1".into(),
            compatibility_entry: "entry".into(),
        },
        prompt: "prompt".into(),
        goal: None,
        fresh_context: None,
        resume_session_id: None,
    }
}
