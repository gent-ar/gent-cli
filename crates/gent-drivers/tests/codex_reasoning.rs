use gent_drivers::codex_session::{CodexSessionConfig, CodexTurnOptions};
use gent_drivers::codex_turn::{CodexTurnDriver, CodexTurnEffect};
use gent_drivers::public_protocol::PublicWireFact;
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, NormalizedProviderEvent,
};
use serde_json::{Value, json};

fn driver() -> CodexTurnDriver {
    CodexTurnDriver::start(
        CodexSessionConfig {
            working_directory: None,
            resume_thread_id: None,
            turn_options: CodexTurnOptions::from_selection(
                &AgentChatSelection {
                    provider: AgentChatProvider::Codex,
                    model: "gpt-5.6".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
                None,
            )
            .unwrap(),
            mcp_servers: None,
        },
        "hello",
        None,
    )
    .unwrap()
    .0
}

fn receive(driver: &mut CodexTurnDriver, frame: Value) -> Vec<String> {
    driver
        .receive(&serde_json::to_vec(&frame).unwrap())
        .unwrap()
        .into_iter()
        .filter_map(|effect| match effect {
            CodexTurnEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::Thinking {
                text,
                ..
            })) => Some(text),
            _ => None,
        })
        .collect()
}

fn completed(id: &str, thread: &str, turn: &str) -> Value {
    json!({"method": "item/completed", "params": {
        "threadId": thread, "turnId": turn,
        "item": {"type": "reasoning", "id": id,
            "content": [{"type": "reasoning_text", "text": "Transcript"}],
            "summary": [{"type": "summary_text", "text": "Summary"}],
            "futureMetadata": {"secret": "must not become output"}}
    }})
}

fn delta(id: &str, channel: &str, text: &str) -> Value {
    let mut frame = json!({"method": format!("item/reasoning/{channel}"), "params": {
        "threadId": "thread-1", "turnId": "turn-1", "itemId": id, "delta": text
    }});
    frame["params"][if channel == "textDelta" {
        "contentIndex"
    } else {
        "summaryIndex"
    }] = json!(0);
    frame
}

#[test]
fn completed_reasoning_retains_both_exposed_channels_and_ignores_metadata() {
    let mut driver = driver();
    assert_eq!(
        receive(&mut driver, completed("item-1", "thread-1", "turn-1")),
        ["Transcript", "\n\nSummary"]
    );
    assert!(receive(&mut driver, completed("item-1", "thread-1", "turn-1")).is_empty());
}

#[test]
fn streamed_summary_does_not_suppress_unstreamed_reasoning_transcript() {
    let mut driver = driver();
    assert_eq!(
        receive(&mut driver, delta("item-1", "summaryTextDelta", "Summary")),
        ["Summary"]
    );
    assert_eq!(
        receive(&mut driver, completed("item-1", "thread-1", "turn-1")),
        ["\n\nTranscript"]
    );
}

#[test]
fn streamed_reasoning_does_not_suppress_unstreamed_summary() {
    let mut driver = driver();
    assert_eq!(
        receive(&mut driver, delta("item-1", "textDelta", "Transcript")),
        ["Transcript"]
    );
    assert_eq!(
        receive(&mut driver, completed("item-1", "thread-1", "turn-1")),
        ["\n\nSummary"]
    );
}

#[test]
fn fully_streamed_reasoning_is_not_duplicated_by_completion() {
    let mut driver = driver();
    receive(&mut driver, delta("item-1", "textDelta", "Transcript"));
    receive(&mut driver, delta("item-1", "summaryTextDelta", "Summary"));
    assert!(receive(&mut driver, completed("item-1", "thread-1", "turn-1")).is_empty());
    assert_eq!(
        receive(&mut driver, completed("item-2", "thread-1", "turn-1")),
        ["\n\nTranscript", "\n\nSummary"]
    );
    assert_eq!(
        receive(&mut driver, completed("item-1", "child-thread", "turn-1")),
        ["Transcript", "\n\nSummary"]
    );
    assert_eq!(
        receive(&mut driver, completed("item-1", "thread-1", "turn-2")),
        ["Transcript", "\n\nSummary"]
    );
}

#[test]
fn repeated_equal_parts_remain_distinct_and_text_wins_over_duplicate_content() {
    let mut driver = driver();
    let mut frame = completed("item-1", "thread-1", "turn-1");
    frame["params"]["item"]["text"] = json!(["same", "same"]);
    frame["params"]["item"]["summary"] = json!([]);
    assert_eq!(receive(&mut driver, frame), ["same", "\n\nsame"]);
}

#[test]
fn a_new_turn_preserves_late_previous_item_tracking_without_cross_turn_suppression() {
    let mut driver = driver();
    receive(&mut driver, delta("item-1", "textDelta", "Transcript"));
    receive(&mut driver, delta("item-1", "summaryTextDelta", "Summary"));
    receive(
        &mut driver,
        json!({"method": "turn/started", "params": {
            "threadId": "thread-1", "turn": {"id": "turn-2"}
        }}),
    );
    assert!(receive(&mut driver, completed("item-1", "thread-1", "turn-1")).is_empty());
    assert_eq!(
        receive(&mut driver, completed("item-1", "thread-1", "turn-2")),
        ["Transcript", "\n\nSummary"]
    );
}

#[test]
fn incomplete_reasoning_identity_is_diagnostic_without_metadata_fallback() {
    let mut driver = driver();
    let mut frame = completed("item-1", "thread-1", "turn-1");
    frame["params"]["item"]["id"] = json!("");
    let effects = driver
        .receive(&serde_json::to_vec(&frame).unwrap())
        .unwrap();
    assert!(matches!(effects.as_slice(), [
        CodexTurnEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::TransportDiagnostic {
            classification,
        }))
    ] if classification == "uncorrelatedCodexReasoning"));
}

#[test]
fn child_usage_does_not_replace_root_context_or_token_totals() {
    let mut driver = driver();
    receive(
        &mut driver,
        json!({"method":"item/completed", "params": {"item": {
            "type":"subAgentActivity", "kind":"started", "id":"tool-parent", "agentThreadId":"child-thread"
        }}}),
    );
    let usage = |thread| {
        json!({"method":"thread/tokenUsage/updated", "params": {
            "threadId":thread, "tokenUsage":{"last":{"inputTokens":12,"outputTokens":3}}
        }})
    };
    let child = driver
        .receive(&serde_json::to_vec(&usage("child-thread")).unwrap())
        .unwrap();
    assert!(child.is_empty());
    let root = driver
        .receive(&serde_json::to_vec(&usage("root-thread")).unwrap())
        .unwrap();
    assert!(root.iter().any(|effect| matches!(effect,
        CodexTurnEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::TokenUsage { usage }))
            if usage.input_tokens == 12 && usage.output_tokens == 3)));
}
