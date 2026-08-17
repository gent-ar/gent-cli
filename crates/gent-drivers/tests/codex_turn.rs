use gent_drivers::codex_session::CodexSessionConfig;
use gent_drivers::codex_turn::{CodexTurnDriver, CodexTurnEffect};
use gent_drivers::public_protocol::PublicWireFact;
use gent_types::NormalizedProviderEvent;
use serde_json::{Value, json};

fn config() -> CodexSessionConfig {
    CodexSessionConfig {
        working_directory: Some("/work".into()),
        resume_thread_id: None,
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
            json!({"id":3,"method":"turn/start","params":{"threadId":"thread-private","input":[{"type":"text","text":"hello"}]}})
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
