use gent_drivers::{
    PublicProvider,
    public_protocol::{PublicWireFact, normalize_public_frame, replay_public_frames},
};
use gent_types::{
    NormalizedLifecycleSignal, NormalizedProviderEvent, RootActivity, ToolPhase, TurnPhase,
};
use serde_json::json;

#[test]
fn claude_stream_json_init_assistant_tool_and_result_are_normalized() {
    let facts = replay_public_frames(
        PublicProvider::Claude,
        &[
            json!({"type":"system","subtype":"init","session_id":"claude-1"}),
            json!({"type":"assistant","message":{"content":[{"type":"text","text":"hello"},{"type":"tool_use","id":"tool-1","name":"Read"}]}}),
            json!({"type":"result","is_error":false}),
        ],
    );
    assert!(facts.contains(&PublicWireFact::SessionStarted {
        provider_session_id: "claude-1".into()
    }));
    assert!(
        facts.contains(&PublicWireFact::Event(NormalizedProviderEvent::Output {
            text: "hello".into(),
            is_partial: false,
        }))
    );
    assert!(facts.contains(&PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::RootPhase {
            phase: TurnPhase::Ready
        }
    )));
    assert!(facts.iter().any(|fact| matches!(fact, PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity }) if activity.tool_use_id == "tool-1" && activity.phase == ToolPhase::Started)));
}

#[test]
fn claude_nested_stream_events_preserve_text_thinking_and_tool_boundaries() {
    let facts = replay_public_frames(
        PublicProvider::Claude,
        &[
            json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"private"}}}),
            json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"hel"}}}),
            json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"consider"}}}),
            json!({"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"tool_use","id":"tool-1","name":"Read"}}}),
            json!({"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"x"}}}),
            json!({"type":"stream_event","event":{"type":"message_stop"}}),
        ],
    );
    assert!(facts.contains(&PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::RootActivity {
            activity: RootActivity::Generating,
        }
    )));
    assert!(
        facts.contains(&PublicWireFact::Event(NormalizedProviderEvent::Output {
            text: "hel".into(),
            is_partial: true,
        }))
    );
    assert!(
        facts.contains(&PublicWireFact::Event(NormalizedProviderEvent::Thinking {
            text: "consider".into(),
            is_partial: true,
        }))
    );
    assert!(facts.iter().any(|fact| matches!(fact, PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity }) if activity.tool_use_id == "tool-1" && activity.phase == ToolPhase::Started)));
    assert!(!format!("{facts:?}").contains("path"));
}

#[test]
fn codex_app_server_turns_messages_tools_and_approvals_are_normalized() {
    let facts = replay_public_frames(
        PublicProvider::Codex,
        &[
            json!({"method":"thread/started","params":{"thread":{"id":"thread-1"}}}),
            json!({"method":"turn/started","params":{"turn":{"id":"turn-1"}}}),
            json!({"method":"item/agentMessage/delta","params":{"delta":"hello"}}),
            json!({"method":"item/completed","params":{"item":{"type":"agentMessage","id":"message-1","text":"hello"}}}),
            json!({"method":"item/started","params":{"item":{"type":"commandExecution","id":"tool-1"}}}),
            json!({"method":"item/commandExecution/requestApproval","params":{}}),
            json!({"method":"turn/completed","params":{"turn":{"id":"turn-1","status":"completed"}}}),
        ],
    );
    assert!(facts.contains(&PublicWireFact::SessionStarted {
        provider_session_id: "thread-1".into()
    }));
    assert!(facts.contains(&PublicWireFact::Event(
        NormalizedProviderEvent::TurnStarted {
            turn_id: "turn-1".into()
        }
    )));
    assert!(
        facts.contains(&PublicWireFact::Event(NormalizedProviderEvent::Output {
            text: "hello".into(),
            is_partial: false,
        }))
    );
    assert!(
        facts.contains(&PublicWireFact::Event(NormalizedProviderEvent::Output {
            text: "hello".into(),
            is_partial: true,
        }))
    );
    assert!(facts.contains(&PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::AttentionRequired
    )));
    assert!(
        facts.contains(&PublicWireFact::Event(NormalizedProviderEvent::TurnEnded {
            turn_id: "turn-1".into()
        }))
    );
}

#[test]
fn malformed_or_unknown_provider_frames_are_safe_diagnostics() {
    for (provider, frame, classification) in [
        (
            PublicProvider::Claude,
            json!({"type":"system","subtype":"init"}),
            "malformedClaudeInit",
        ),
        (
            PublicProvider::Codex,
            json!({"method":"turn/completed","params":{"turn":{"id":"a","status":"future"}}}),
            "malformedCodexTurn",
        ),
    ] {
        assert_eq!(
            normalize_public_frame(provider, &frame),
            vec![PublicWireFact::Event(
                NormalizedProviderEvent::TransportDiagnostic {
                    classification: classification.into()
                }
            )]
        );
    }
}
