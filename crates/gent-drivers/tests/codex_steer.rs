use gent_drivers::codex_session::{CodexSessionConfig, CodexSteerOutcome, CodexTurnOptions};
use gent_drivers::codex_turn::{CodexTurnDriver, CodexTurnEffect};
use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection};
use serde_json::{Value, json};

fn live_turn() -> CodexTurnDriver {
    let config = CodexSessionConfig {
        working_directory: Some("/work".into()),
        resume_thread_id: None,
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
    };
    let (mut driver, _) = CodexTurnDriver::start(config, "first", None).unwrap();
    for frame in [
        json!({"id": 1, "result": {}}),
        json!({"id": 2, "result": {"thread": {"id": "thread-private"}}}),
        json!({"method": "turn/started", "params": {"threadId": "thread-private", "turn": {"id": "turn-1"}}}),
        json!({"id": 3, "result": {"turn": {"id": "turn-1"}}}),
    ] {
        driver.receive(frame.to_string().as_bytes()).unwrap();
    }
    driver
}

fn written(effect: CodexTurnEffect) -> Value {
    let CodexTurnEffect::Write(frame) = effect else {
        panic!("a steer must be a provider write");
    };
    serde_json::from_slice(&frame).unwrap()
}

fn steers(driver: &mut CodexTurnDriver, frame: Value) -> Vec<CodexSteerOutcome> {
    driver
        .receive(frame.to_string().as_bytes())
        .unwrap()
        .into_iter()
        .filter_map(|effect| match effect {
            CodexTurnEffect::Steer(outcome) => Some(outcome),
            _ => None,
        })
        .collect()
}

fn consumed(client_id: &str) -> Value {
    json!({"method": "item/started", "params": {"threadId": "thread-private", "turnId": "turn-1", "item": {"type": "userMessage", "id": "item", "clientId": client_id, "content": []}}})
}

#[test]
fn a_steer_targets_the_live_turn_and_reports_consumption_by_its_gent_message() {
    let mut driver = live_turn();
    let frame = written(driver.steer("message-1", "also this", &[]).unwrap());
    assert_eq!(
        frame,
        json!({"jsonrpc": "2.0", "id": 4, "method": "turn/steer", "params": {
            "threadId": "thread-private", "expectedTurnId": "turn-1",
            "input": [{"type": "text", "text": "also this"}], "clientUserMessageId": "message-1"
        }})
    );
    assert!(
        steers(
            &mut driver,
            json!({"id": 4, "result": {"turnId": "turn-1"}})
        )
        .is_empty()
    );
    assert!(steers(&mut driver, consumed("someone-else")).is_empty());
    assert_eq!(
        steers(&mut driver, consumed("message-1")),
        [CodexSteerOutcome::Consumed {
            message_id: "message-1".into()
        }]
    );
    assert!(steers(&mut driver, consumed("message-1")).is_empty());
}

#[test]
fn a_rejected_steer_is_reported_without_failing_the_session() {
    let mut driver = live_turn();
    driver.steer("message-1", "too late", &[]).unwrap();
    assert_eq!(
        steers(
            &mut driver,
            json!({"id": 4, "error": {"code": -32600, "message": "no active turn to steer"}})
        ),
        [CodexSteerOutcome::Rejected {
            message_id: "message-1".into()
        }]
    );
    driver
        .receive(br#"{"method":"turn/completed","params":{"threadId":"thread-private","turn":{"id":"turn-1"}}}"#)
        .unwrap();
    assert!(driver.submit("next", None, &[], None).is_ok());
}

#[test]
fn an_accepted_steer_the_turn_ends_without_is_returned_to_gent() {
    let mut driver = live_turn();
    driver.steer("accepted", "held", &[]).unwrap();
    driver.steer("unanswered", "held", &[]).unwrap();
    steers(
        &mut driver,
        json!({"id": 4, "result": {"turnId": "turn-1"}}),
    );
    let ended = json!({"method": "turn/completed", "params": {"threadId": "thread-private", "turn": {"id": "turn-1", "status": "interrupted"}}});
    assert_eq!(
        steers(&mut driver, ended),
        [CodexSteerOutcome::Rejected {
            message_id: "accepted".into()
        }]
    );
    assert_eq!(
        steers(
            &mut driver,
            json!({"id": 5, "result": {"turnId": "turn-1"}})
        ),
        [CodexSteerOutcome::Rejected {
            message_id: "unanswered".into()
        }]
    );
    assert!(driver.steer("idle", "no turn", &[]).is_err());
}
