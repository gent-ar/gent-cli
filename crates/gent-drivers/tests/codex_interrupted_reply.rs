use gent_drivers::codex_session::{CodexSessionConfig, CodexTurnOptions};
use gent_drivers::codex_turn::{CodexTurnDriver, CodexTurnEffect};
use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection};
use serde_json::{Value, json};

const PARTIAL: &str = "The codeword is ZEBRA-4821 and the story begins";

fn config(resume_thread_id: Option<&str>) -> CodexSessionConfig {
    CodexSessionConfig {
        working_directory: Some("/work".into()),
        resume_thread_id: resume_thread_id.map(str::to_owned),
        turn_options: CodexTurnOptions::from_selection(
            &AgentChatSelection {
                provider: AgentChatProvider::Codex,
                model: "gpt-5.6".into(),
                effort: AgentChatEffort::Medium,
                mode: AgentChatMode::Agent,
            },
            Some("/work"),
        )
        .unwrap(),
        mcp_servers: None,
    }
}

fn frames(effects: &[CodexTurnEffect]) -> Vec<Value> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            CodexTurnEffect::Write(frame) => serde_json::from_slice(&frame[..frame.len() - 1]).ok(),
            _ => None,
        })
        .collect()
}

fn ready_after_one_turn() -> CodexTurnDriver {
    let (mut driver, _) = CodexTurnDriver::start(config(None), "first", None).unwrap();
    for frame in [
        r#"{"id":1,"result":{}}"#,
        r#"{"id":2,"result":{"thread":{"id":"thread-private"}}}"#,
        r#"{"id":3,"result":{"turn":{"id":"turn-1"}}}"#,
        r#"{"method":"turn/completed","params":{"threadId":"thread-private","turn":{"id":"turn-1"}}}"#,
    ] {
        driver.receive(frame.as_bytes()).unwrap();
    }
    driver
}

fn turn_text(frame: &Value) -> &str {
    assert_eq!(frame["method"], "turn/start");
    frame["params"]["input"][0]["text"].as_str().unwrap()
}

#[test]
fn a_live_thread_restores_its_interrupted_reply_before_the_next_turn() {
    let mut driver = ready_after_one_turn();
    let inject = frames(
        &driver
            .submit("what was the codeword?", None, &[], Some(PARTIAL))
            .unwrap(),
    );
    assert_eq!(
        inject,
        [json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "thread/inject_items",
            "params": {
                "threadId": "thread-private",
                "items": [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": PARTIAL}]}]
            }
        })]
    );
    assert!(driver.submit("too early", None, &[], None).is_err());
    let turn = frames(&driver.receive(br#"{"id":4,"result":{}}"#).unwrap());
    assert_eq!(turn.len(), 1);
    assert_eq!(turn[0]["id"], 5);
    assert_eq!(turn_text(&turn[0]), "what was the codeword?");
}

#[test]
fn a_thread_without_inject_items_receives_the_reply_as_tagged_prior_context() {
    let mut driver = ready_after_one_turn();
    driver
        .submit("what was the codeword?", None, &[], Some(PARTIAL))
        .unwrap();
    let turn = frames(
        &driver
            .receive(br#"{"id":4,"error":{"code":-32601,"message":"Method not found"}}"#)
            .unwrap(),
    );
    assert_eq!(turn.len(), 1);
    let text = turn_text(&turn[0]);
    assert!(text.contains(&format!("\n{PARTIAL}\n")), "{text}");
    assert!(text.contains("[Assistant, interrupted before finishing · "));
    assert!(text.ends_with("Current user prompt:\nwhat was the codeword?"));
    driver
        .receive(br#"{"id":5,"result":{"turn":{"id":"turn-2"}}}"#)
        .unwrap();
    driver
        .receive(br#"{"method":"turn/completed","params":{"threadId":"thread-private","turn":{"id":"turn-2"}}}"#)
        .unwrap();
    assert_eq!(
        turn_text(&frames(&driver.submit("next", None, &[], None).unwrap())[0]),
        "next"
    );
}

#[test]
fn a_resumed_thread_restores_the_interrupted_reply_before_its_first_turn() {
    let (mut driver, _) = CodexTurnDriver::start_with_attachments(
        config(Some("thread-private")),
        "what was the codeword?",
        Vec::new(),
        None,
        Some(PARTIAL.into()),
    )
    .unwrap();
    driver.receive(br#"{"id":1,"result":{}}"#).unwrap();
    let ready = frames(
        &driver
            .receive(br#"{"id":2,"result":{"thread":{"id":"thread-private"}}}"#)
            .unwrap(),
    );
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0]["method"], "thread/inject_items");
    assert_eq!(
        ready[0]["params"]["items"][0]["content"][0]["text"],
        PARTIAL
    );
    let turn = frames(&driver.receive(br#"{"id":3,"result":{}}"#).unwrap());
    assert_eq!(turn_text(&turn[0]), "what was the codeword?");
}

#[test]
fn a_blank_interrupted_reply_starts_the_turn_directly() {
    let mut driver = ready_after_one_turn();
    let turn = frames(&driver.submit("next", None, &[], Some("  \n")).unwrap());
    assert_eq!(turn_text(&turn[0]), "next");
}
