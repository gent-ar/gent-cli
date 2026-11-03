use gent_drivers::{
    PublicProvider,
    public_protocol::{PublicWireFact, normalize_public_frame},
};
use gent_types::{
    AgentChatCompactionFailure, NormalizedLifecycleSignal, NormalizedProviderEvent,
    ProviderFailureClassification, ToolPhase, TurnPhase,
};
use serde_json::{Value, json};

#[path = "public_protocol_edges/codex_items.rs"]
mod codex_items;
mod public_protocol_edges_replay;

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
    for method in ["turn/aborted", "turn/failed"] {
        assert_eq!(
            facts(
                PublicProvider::Codex,
                &json!({"method":method,"params":{"turnId":"turn"}}),
            ),
            vec![PublicWireFact::Event(
                NormalizedProviderEvent::TransportDiagnostic {
                    classification: "unsupportedCodexNotification".into()
                }
            )]
        );
    }
    for kind in ["fileChange", "mcpToolCall"] {
        assert!(facts(PublicProvider::Codex, &json!({"method":"item/completed","params":{"item":{"type":kind,"id":"tool"}}})).iter().any(
            |fact| matches!(fact, PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity }) if activity.tool_name == kind && activity.phase == ToolPhase::Completed)
        ));
    }
    assert!(facts(
        PublicProvider::Codex,
        &json!({"method":"item/started","params":{"item":{"type":"mcpToolCall","id":"tool","server":"linear-cloud","tool":"search_issues"}}}),
    )
    .iter()
    .any(|fact| matches!(fact, PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity }) if activity.tool_name == "mcp__linear_cloud__search_issues" && activity.phase == ToolPhase::Started)));
    assert!(facts(
        PublicProvider::Codex,
        &json!({"method":"item/mcpToolCall/progress","params":{"itemId":"tool","message":"connected"}}),
    )
    .iter()
    .any(|fact| matches!(fact, PublicWireFact::Event(NormalizedProviderEvent::ToolOutputDelta { tool_use_id, text, is_partial }) if tool_use_id == "tool" && text == "connected" && *is_partial)));
    assert!(
        facts(
            PublicProvider::Codex,
            &json!({"method":"item/started","params":{"item":{"type":"agentMessage","id":"item"}}})
        )
        .is_empty()
    );
    assert!(
        facts(
            PublicProvider::Codex,
            &json!({"method":"remoteControl/status/changed","params":{"status":"disabled"}})
        )
        .is_empty(),
        "Codex emits remote-control transport state between initialize and thread/started"
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
            json!({"method":"item/completed","params":{"item":{"type":"agentMessage"}}}),
            "malformedCodexAgentMessage",
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
fn codex_terminal_failure_persists_only_a_redacted_classification_and_message() {
    let facts = facts(
        PublicProvider::Codex,
        &json!({
            "method":"turn/completed",
            "params":{"turn":{"id":"turn", "status":"failed", "error":{
                "codexErrorInfo":"unauthorized", "message":"Bearer sk-secret and /private/path", "additionalDetails":null
            }}}
        }),
    );
    assert!(facts.contains(&PublicWireFact::Event(
        NormalizedProviderEvent::ProviderFailure {
            classification: ProviderFailureClassification::Authentication,
            message: "Codex authentication failed.".into(),
        },
    )));
    assert!(!format!("{facts:?}").contains("sk-secret"));
    assert!(!format!("{facts:?}").contains("private/path"));
}

#[test]
fn codex_known_transport_notifications_reduce_to_no_public_fact() {
    for method in [
        "mcpServer/startupStatus/updated",
        "warning",
        "guardianWarning",
        "deprecationNotice",
        "configWarning",
        "model/rerouted",
        "model/verification",
        "serverRequest/resolved",
        "thread/archived",
        "thread/unarchived",
        "thread/deleted",
        "thread/closed",
        "thread/goal/updated",
        "thread/goal/cleared",
        "account/updated",
        "model/safetyBuffering/updated",
        "account/login/completed",
        "hook/started",
        "hook/completed",
        "mcpServer/oauthLogin/completed",
        "windows/worldWritableWarning",
        "windowsSandbox/setupCompleted",
        "item/autoApprovalReview/started",
        "item/autoApprovalReview/completed",
        "thread/realtime/started",
        "thread/realtime/closed",
        "thread/realtime/error",
        "thread/realtime/sdp",
        "app/list/updated",
        "fs/changed",
        "process/outputDelta",
        "process/exited",
        "externalAgentConfig/import/progress",
        "externalAgentConfig/import/completed",
    ] {
        assert!(
            facts(PublicProvider::Codex, &json!({"method": method})).is_empty(),
            "{method}"
        );
    }
}

#[test]
fn claude_manual_compaction_frames_from_2_1_270_become_compaction_observations() {
    use gent_drivers::public_protocol::PublicCompactionObservation;
    let frames = [
        json!({"type":"system","subtype":"status","status":"compacting","session_id":"secret"}),
        json!({"type":"system","subtype":"status","status":null,"compact_result":"success","session_id":"secret"}),
        json!({"type":"system","subtype":"compact_boundary","session_id":"secret","compact_metadata":{"trigger":"manual","pre_tokens":23667,"post_tokens":1579}}),
        json!({"type":"user","message":{"role":"user","content":"This session is being continued from a previous conversation."},"isSynthetic":true}),
        json!({"type":"system","subtype":"status","status":null,"compact_result":"failed","session_id":"secret"}),
        json!({"type":"system","subtype":"status","status":null,"session_id":"secret"}),
    ];
    let facts: Vec<_> = frames
        .iter()
        .flat_map(|frame| normalize_public_frame(PublicProvider::Claude, frame))
        .collect();
    assert_eq!(
        facts,
        [
            PublicWireFact::Compaction(PublicCompactionObservation::Started),
            PublicWireFact::Compaction(PublicCompactionObservation::Completed),
            PublicWireFact::Compaction(PublicCompactionObservation::Failed {
                failure: gent_types::AgentChatCompactionFailure::ProviderFailed,
            }),
        ]
    );
}
