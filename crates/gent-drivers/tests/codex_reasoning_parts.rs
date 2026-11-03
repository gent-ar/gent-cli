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

fn delta(item: &str, turn: &str, channel: &str, index: u64, text: &str) -> Value {
    let mut frame = json!({"method":format!("item/reasoning/{channel}"),"params":{
        "threadId":"thread-1","turnId":turn,"itemId":item,"delta":text}});
    frame["params"][if channel == "textDelta" {
        "contentIndex"
    } else {
        "summaryIndex"
    }] = json!(index);
    frame
}

fn completed(item: &str, turn: &str, summary: Value, content: Value) -> Value {
    json!({"method":"item/completed","params":{"threadId":"thread-1","turnId":turn,
        "item":{"id":item,"type":"reasoning","summary":summary,"content":content}}})
}

#[test]
fn same_native_part_chunks_concatenate_without_inserted_whitespace() {
    let mut driver = driver();
    assert_eq!(
        receive(
            &mut driver,
            delta("item-1", "turn-1", "summaryTextDelta", 0, "**Plan")
        ),
        ["**Plan"]
    );
    assert_eq!(
        receive(
            &mut driver,
            delta("item-1", "turn-1", "summaryTextDelta", 0, "**\nBody")
        ),
        ["**\nBody"]
    );
}

#[test]
fn new_summary_part_preserves_its_paragraph_boundary() {
    let mut driver = driver();
    receive(
        &mut driver,
        delta(
            "item-1",
            "turn-1",
            "summaryTextDelta",
            0,
            "**Investigating patch newline handling**",
        ),
    );
    assert!(receive(&mut driver,json!({"method":"item/reasoning/summaryPartAdded","params":{"threadId":"thread-1","turnId":"turn-1","itemId":"item-1","summaryIndex":1}})).is_empty());
    assert_eq!(
        receive(
            &mut driver,
            delta(
                "item-1",
                "turn-1",
                "summaryTextDelta",
                1,
                "**Planning no-newline file creation with apply_patch**"
            )
        ),
        ["\n\n**Planning no-newline file creation with apply_patch**"]
    );
}

#[test]
fn different_items_with_equal_text_remain_distinct_native_blocks() {
    let mut driver = driver();
    assert_eq!(
        receive(
            &mut driver,
            delta("item-1", "turn-1", "summaryTextDelta", 0, "Same")
        ),
        ["Same"]
    );
    assert_eq!(
        receive(
            &mut driver,
            delta("item-2", "turn-1", "summaryTextDelta", 0, "Same")
        ),
        ["\n\nSame"]
    );
}

#[test]
fn completed_summary_preserves_each_unstreamed_part() {
    let mut driver = driver();
    receive(
        &mut driver,
        delta("item-1", "turn-1", "summaryTextDelta", 0, "Streamed"),
    );
    assert_eq!(
        receive(
            &mut driver,
            completed(
                "item-1",
                "turn-1",
                json!(["Streamed", "Unstreamed"]),
                json!([])
            )
        ),
        ["\n\nUnstreamed"]
    );
    assert!(
        receive(
            &mut driver,
            completed(
                "item-1",
                "turn-1",
                json!(["Streamed", "Unstreamed"]),
                json!([])
            )
        )
        .is_empty()
    );
}

#[test]
fn completed_text_preserves_unstreamed_content_index_and_channel_boundary() {
    let mut driver = driver();
    receive(
        &mut driver,
        delta("item-1", "turn-1", "textDelta", 0, "First"),
    );
    assert_eq!(
        receive(
            &mut driver,
            completed(
                "item-1",
                "turn-1",
                json!(["Summary"]),
                json!(["First", "Second"])
            )
        ),
        ["\n\nSecond", "\n\nSummary"]
    );
}

#[test]
fn late_old_turn_part_recovery_does_not_use_new_turn_block_state() {
    let mut driver = driver();
    receive(
        &mut driver,
        delta("item-1", "turn-1", "summaryTextDelta", 0, "Old first"),
    );
    receive(
        &mut driver,
        json!({"method":"turn/started","params":{"threadId":"thread-1","turn":{"id":"turn-2"}}}),
    );
    assert_eq!(
        receive(
            &mut driver,
            delta("item-1", "turn-2", "summaryTextDelta", 0, "New first")
        ),
        ["New first"]
    );
    assert_eq!(
        receive(
            &mut driver,
            completed(
                "item-1",
                "turn-1",
                json!(["Old first", "Old second"]),
                json!([])
            )
        ),
        ["\n\nOld second"]
    );
}

#[test]
fn completed_only_parts_are_separated_without_text_based_deduplication() {
    let mut driver = driver();
    assert_eq!(
        receive(
            &mut driver,
            completed("item-1", "turn-1", json!(["Same", "Same"]), json!([]))
        ),
        ["Same", "\n\nSame"]
    );
}

#[test]
fn part_tracking_is_finite_and_existing_part_chunks_still_work_at_the_bound() {
    let mut driver = driver();
    for index in 0..256 {
        receive(
            &mut driver,
            delta("item-1", "turn-1", "summaryTextDelta", index, "part"),
        );
    }
    let effects = driver
        .receive(
            &serde_json::to_vec(&delta(
                "item-1",
                "turn-1",
                "summaryTextDelta",
                256,
                "over bound",
            ))
            .unwrap(),
        )
        .unwrap();
    assert!(
        matches!(effects.as_slice(),[CodexTurnEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::TransportDiagnostic { classification }))] if classification == "codexReasoningPartLimit")
    );
    assert_eq!(
        receive(
            &mut driver,
            delta("item-1", "turn-1", "summaryTextDelta", 255, " continued")
        ),
        [" continued"]
    );
}

#[test]
fn malformed_part_index_is_diagnostic_and_does_not_poison_the_next_part() {
    let mut driver = driver();
    let mut malformed = delta("item-1", "turn-1", "summaryTextDelta", 0, "invalid");
    malformed["params"]["summaryIndex"] = json!(-1);
    let effects = driver
        .receive(&serde_json::to_vec(&malformed).unwrap())
        .unwrap();
    assert!(
        matches!(effects.as_slice(),[CodexTurnEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::TransportDiagnostic { classification }))] if classification == "malformedCodexReasoningPart")
    );
    assert_eq!(
        receive(
            &mut driver,
            delta("item-1", "turn-1", "summaryTextDelta", 0, "valid")
        ),
        ["valid"]
    );
}
