use gent_drivers::{
    PublicProvider,
    public_protocol::{PublicWireFact, normalize_public_frame},
};
use gent_types::{NormalizedLifecycleSignal, NormalizedProviderEvent, ToolPhase, TurnPhase};
use serde_json::{Value, json};

fn facts(provider: PublicProvider, frame: &Value) -> Vec<PublicWireFact> {
    normalize_public_frame(provider, frame)
}

fn diagnostic(provider: PublicProvider, frame: &Value, expected: &str) {
    assert_eq!(
        facts(provider, frame),
        vec![PublicWireFact::Event(
            NormalizedProviderEvent::TransportDiagnostic {
                classification: expected.into(),
            },
        )]
    );
}

#[test]
fn claude_rejects_incomplete_content_without_retaining_it() {
    for (frame, expected) in [
        (json!({"type":"assistant"}), "malformedClaudeAssistant"),
        (
            json!({"type":"assistant","message":{"content":[]}}),
            "emptyClaudeAssistant",
        ),
        (
            json!({"type":"assistant","message":{"content":[{"type":"text"}]}}),
            "malformedClaudeText",
        ),
        (
            json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"x"}]}}),
            "malformedClaudeToolUse",
        ),
        (
            json!({"type":"assistant","message":{"content":[{"type":"image"}]}}),
            "unsupportedClaudeContent",
        ),
        (json!({"type":"future"}), "unsupportedClaudeFrame"),
    ] {
        diagnostic(PublicProvider::Claude, &frame, expected);
    }
    assert!(
        facts(
            PublicProvider::Claude,
            &json!({"type":"result","is_error":true})
        )
        .contains(&PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::RootPhase {
                phase: TurnPhase::Failed
            }
        ))
    );
}

#[test]
fn codex_covers_terminal_statuses_tools_and_unknown_frames() {
    for (status, phase) in [
        ("interrupted", TurnPhase::Interrupted),
        ("failed", TurnPhase::Failed),
    ] {
        assert!(
            facts(
                PublicProvider::Codex,
                &json!({"method":"turn/completed","params":{"turn":{"id":"turn", "status":status}}})
            )
            .contains(&PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::RootPhase { phase }
            ))
        );
    }
    for kind in ["fileChange", "mcpToolCall"] {
        assert!(facts(PublicProvider::Codex, &json!({"method":"item/completed","params":{"item":{"type":kind,"id":"tool"}}})).iter().any(
            |fact| matches!(fact, PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity }) if activity.tool_name == kind && activity.phase == ToolPhase::Completed)
        ));
    }
    assert!(
        facts(
            PublicProvider::Codex,
            &json!({"method":"item/started","params":{"item":{"type":"agentMessage","id":"item"}}})
        )
        .is_empty()
    );
    for (frame, expected) in [
        (
            json!({"method":"thread/started","params":{"thread":{}}}),
            "malformedCodexThread",
        ),
        (
            json!({"method":"turn/started","params":{}}),
            "malformedCodexTurn",
        ),
        (
            json!({"method":"item/agentMessage/delta","params":{}}),
            "malformedCodexMessageDelta",
        ),
        (
            json!({"method":"item/started","params":{"item":{}}}),
            "malformedCodexItem",
        ),
        (json!({"method":"future"}), "unsupportedCodexNotification"),
        (json!({"error":{"code":-1}}), "codexRpcError"),
        (json!({"jsonrpc":"2.0"}), "malformedCodexFrame"),
    ] {
        diagnostic(PublicProvider::Codex, &frame, expected);
    }
}
