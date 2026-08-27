use gent_drivers::{
    PublicProvider,
    public_protocol::{PublicWireFact, normalize_public_frame},
};
use gent_types::{
    AgentChatCompactionFailure, NormalizedLifecycleSignal, NormalizedProviderEvent,
    ProviderFailureClassification, ToolPhase, TurnPhase,
};
use serde_json::{Value, json};

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
    for (method, phase) in [
        ("turn/aborted", TurnPhase::Interrupted),
        ("turn/failed", TurnPhase::Failed),
    ] {
        assert!(
            facts(
                PublicProvider::Codex,
                &json!({"method":method,"params":{"turnId":"turn"}}),
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
                "code":"unauthorized", "message":"Bearer sk-secret and /private/path"
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
fn codex_current_item_states_and_plan_compaction_remain_content_safe() {
    for kind in [
        "dynamicToolCall",
        "collabAgentToolCall",
        "webSearch",
        "imageGeneration",
    ] {
        let started = facts(
            PublicProvider::Codex,
            &json!({"method":"item/started","params":{"item":{"type":kind,"id":"tool"}}}),
        );
        assert!(started.iter().any(|fact| matches!(fact, PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity }) if activity.tool_name == kind && activity.phase == ToolPhase::Started)));
    }
    let failed = facts(
        PublicProvider::Codex,
        &json!({"method":"item/completed","params":{"item":{"type":"commandExecution","id":"tool","status":"failed","command":"secret"}}}),
    );
    assert!(failed.iter().any(|fact| matches!(fact, PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity }) if activity.phase == ToolPhase::Failed)));
    let frame = json!({"method":"thread/compacted","params":{"threadId":"secret-thread"}});
    let normalized = facts(PublicProvider::Codex, &frame);
    assert_eq!(
        normalized,
        vec![PublicWireFact::Compaction(
            gent_drivers::public_protocol::PublicCompactionObservation::Completed
        )]
    );
    assert!(!format!("{normalized:?}").contains("secret"));
    for method in [
        "item/plan/delta",
        "item/reasoning/textDelta",
        "item/reasoning/summaryTextDelta",
    ] {
        assert_eq!(
            facts(
                PublicProvider::Codex,
                &json!({"method":method,"params":{"delta":"working through it"}}),
            ),
            vec![PublicWireFact::Event(NormalizedProviderEvent::Thinking {
                text: "working through it".into(),
                is_partial: true,
            })],
        );
    }
    assert!(
        facts(
            PublicProvider::Codex,
            &json!({"method":"item/reasoning/summaryPartAdded","params":{}}),
        )
        .is_empty()
    );
    for method in [
        "item/commandExecution/outputDelta",
        "item/commandExecution/terminalInteraction",
        "item/fileChange/outputDelta",
    ] {
        let normalized = facts(
            PublicProvider::Codex,
            &json!({"method":method,"params":{"itemId":"tool","delta":"private"}}),
        );
        assert_eq!(
            normalized,
            vec![PublicWireFact::Event(
                NormalizedProviderEvent::ToolOutputDelta {
                    tool_use_id: "tool".into(),
                    text: "private".into(),
                    is_partial: true,
                }
            )],
            "{method} is a live tool-output frame"
        );
    }
    assert_eq!(
        facts(
            PublicProvider::Codex,
            &json!({"method":"item/fileChange/patchUpdated","params":{"itemId":"tool","changes":[{"diff":"private"}]}}),
        ),
        vec![PublicWireFact::Event(
            NormalizedProviderEvent::ToolOutputDelta {
                tool_use_id: "tool".into(),
                text: "private".into(),
                is_partial: true,
            }
        )]
    );
    assert!(
        facts(
            PublicProvider::Codex,
            &json!({"method":"item/tool/call","params":{"itemId":"tool","delta":"private"}}),
        )
        .is_empty()
    );
    assert_eq!(
        facts(
            PublicProvider::Codex,
            &json!({"method":"turn/plan/updated","params":{"plan":[{"step":"review durable state","status":"inProgress"}]}}),
        ),
        vec![PublicWireFact::Event(NormalizedProviderEvent::Thinking {
            text: "1. review durable state".into(),
            is_partial: false,
        })],
    );
    assert_eq!(
        facts(
            PublicProvider::Codex,
            &json!({"method":"item/started","params":{"item":{"type":"contextCompaction","secret":"ignored"}}}),
        ),
        vec![PublicWireFact::Compaction(
            gent_drivers::public_protocol::PublicCompactionObservation::Started
        )]
    );
    assert_eq!(
        facts(
            PublicProvider::Codex,
            &json!({"method":"item/completed","params":{"item":{"type":"contextCompaction","status":"failed","secret":"ignored"}}}),
        ),
        vec![PublicWireFact::Compaction(
            gent_drivers::public_protocol::PublicCompactionObservation::Failed {
                failure: AgentChatCompactionFailure::ProviderFailed,
            }
        )]
    );
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
        "thread/realtime/transcript",
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
