use super::*;

fn methods(effects: &[CodexTurnEffect]) -> Vec<String> {
    frames(effects)
        .iter()
        .filter_map(|frame| frame["method"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn a_warm_session_reaches_its_thread_without_starting_a_turn() {
    let (mut driver, initial) = CodexTurnDriver::awaiting_prompt(config()).unwrap();
    assert_eq!(methods(&initial), ["initialize"]);
    let handshake = driver.receive(br#"{"id":1,"result":{}}"#).unwrap();
    assert_eq!(methods(&handshake), ["initialized", "thread/start"]);

    let ready = driver
        .receive(br#"{"id":2,"result":{"thread":{"id":"thread-private"}}}"#)
        .unwrap();

    assert!(methods(&ready).is_empty());
    assert!(
        ready.contains(&CodexTurnEffect::Fact(PublicWireFact::SessionStarted {
            provider_session_id: "thread-private".into(),
        }))
    );
    let turn = frames(&driver.submit("hello", None, &[], None).unwrap());
    assert_eq!(turn[0]["method"], "turn/start");
    assert_eq!(turn[0]["params"]["input"][0]["text"], "hello");
}

#[test]
fn a_prompt_submitted_during_the_handshake_starts_once_the_thread_is_ready() {
    let (mut driver, _) = CodexTurnDriver::awaiting_prompt(config()).unwrap();
    assert!(driver.submit("early", None, &[], None).unwrap().is_empty());
    assert!(driver.submit("twice", None, &[], None).is_err());
    driver.receive(br#"{"id":1,"result":{}}"#).unwrap();

    let ready = frames(
        &driver
            .receive(br#"{"id":2,"result":{"thread":{"id":"thread-private"}}}"#)
            .unwrap(),
    );

    assert_eq!(ready[0]["method"], "turn/start");
    assert_eq!(ready[0]["params"]["input"][0]["text"], "early");
}
