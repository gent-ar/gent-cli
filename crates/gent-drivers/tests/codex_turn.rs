use gent_drivers::codex_session::{CodexSessionConfig, CodexTurnOptions};
use gent_drivers::codex_turn::{CodexTurnDriver, CodexTurnEffect};
use gent_drivers::public_protocol::PublicWireFact;
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, NormalizedProviderEvent,
};
use serde_json::{Value, json};

fn config() -> CodexSessionConfig {
    CodexSessionConfig {
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
    }
}

fn frames(effects: &[CodexTurnEffect]) -> Vec<Value> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            CodexTurnEffect::Write(frame) => serde_json::from_slice(&frame[..frame.len() - 1]).ok(),
            CodexTurnEffect::Fact(_) => None,
        })
        .collect()
}

#[test]
fn handshakes_then_starts_the_exact_one_prompt_without_exporting_native_ids() {
    let (mut driver, initial) = CodexTurnDriver::start(config(), "hello".into()).unwrap();
    assert_eq!(frames(&initial)[0]["method"], "initialize");
    let next = driver.receive(br#"{"id":1,"result":{}}"#).unwrap();
    assert_eq!(
        frames(&next)
            .iter()
            .map(|value| value["method"].clone())
            .collect::<Vec<_>>(),
        vec![json!("initialized"), json!("thread/start")]
    );
    let next = driver
        .receive(br#"{"id":2,"result":{"thread":{"id":"thread-private"}}}"#)
        .unwrap();
    assert_eq!(
        frames(&next),
        vec![
            json!({"id":3,"method":"turn/start","params":{"threadId":"thread-private","input":[{"type":"text","text":"hello"}],"model":"gpt-5.6","effort":"medium","approvalPolicy":"untrusted","sandboxPolicy":{"type":"workspaceWrite","writableRoots":["/work"],"networkAccess":false,"excludeTmpdirEnvVar":false,"excludeSlashTmp":false}}})
        ]
    );
}

#[test]
fn normalizes_notifications_but_discards_malformed_notification_state() {
    let (mut driver, _) = CodexTurnDriver::start(config(), "hello".into()).unwrap();
    let malformed = driver
        .receive(br#"{"method":"turn/started","params":{"threadId":"x","turn":{}}}"#)
        .unwrap();
    assert!(matches!(
        malformed.as_slice(),
        [CodexTurnEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::TransportDiagnostic { classification }))]
            if classification == "malformedCodexTurn"
    ));
    let next = driver.receive(br#"{"id":1,"result":{}}"#).unwrap();
    assert_eq!(frames(&next)[1]["method"], "thread/start");
}

#[test]
fn malformed_raw_frame_is_a_normalized_diagnostic_and_oversized_input_is_refused() {
    let (mut driver, _) = CodexTurnDriver::start(config(), "hello".into()).unwrap();
    assert!(matches!(
        driver.receive(b"not-json").unwrap().as_slice(),
        [CodexTurnEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::TransportDiagnostic { classification }))]
            if classification == "malformedCodexFrame"
    ));
    assert!(driver.receive(&vec![b'x'; 65_537]).is_err());
}

#[test]
fn reuses_the_ready_native_thread_for_a_later_prompt() {
    let (mut driver, _) = CodexTurnDriver::start(config(), "first".into()).unwrap();
    driver.receive(br#"{"id":1,"result":{}}"#).unwrap();
    driver
        .receive(br#"{"method":"thread/started","params":{"thread":{"id":"thread-private"}}}"#)
        .unwrap();
    driver
        .receive(br#"{"id":2,"result":{"thread":{"id":"thread-private"}}}"#)
        .unwrap();
    driver
        .receive(br#"{"method":"turn/started","params":{"threadId":"thread-private","turn":{"id":"turn-1"}}}"#)
        .unwrap();
    driver
        .receive(br#"{"id":3,"result":{"turn":{"id":"turn-1"}}}"#)
        .unwrap();
    driver
        .receive(br#"{"method":"turn/completed","params":{"threadId":"thread-private","turn":{"id":"turn-1"}}}"#)
        .unwrap();
    assert_eq!(
        frames(&driver.submit("follow-up").unwrap()),
        vec![
            json!({"id":4,"method":"turn/start","params":{"threadId":"thread-private","input":[{"type":"text","text":"follow-up"}],"model":"gpt-5.6","effort":"medium","approvalPolicy":"untrusted","sandboxPolicy":{"type":"workspaceWrite","writableRoots":["/work"],"networkAccess":false,"excludeTmpdirEnvVar":false,"excludeSlashTmp":false}}})
        ]
    );
}
