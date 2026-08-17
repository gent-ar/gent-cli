use gent_drivers::{
    PublicProvider,
    public_protocol::{PublicWireFact, normalize_public_frame, replay_public_frames},
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

#[test]
fn a_public_parser_fault_does_not_poison_the_following_provider_frame() {
    let claude = replay_public_frames(
        PublicProvider::Claude,
        &[
            json!({"type":"assistant"}),
            json!({"type":"result","is_error":false}),
        ],
    );
    assert!(claude.contains(&PublicWireFact::Event(
        NormalizedProviderEvent::TransportDiagnostic {
            classification: "malformedClaudeAssistant".into(),
        }
    )));
    assert!(claude.contains(&PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::RootPhase {
            phase: TurnPhase::Ready
        }
    )));

    let codex = replay_public_frames(
        PublicProvider::Codex,
        &[
            json!({"jsonrpc":"2.0"}),
            json!({"method":"turn/started","params":{"turn":{"id":"after-fault"}}}),
        ],
    );
    assert!(codex.contains(&PublicWireFact::Event(
        NormalizedProviderEvent::TransportDiagnostic {
            classification: "malformedCodexFrame".into(),
        }
    )));
    assert!(codex.contains(&PublicWireFact::Event(
        NormalizedProviderEvent::TurnStarted {
            turn_id: "after-fault".into(),
        }
    )));
}
