use super::*;

fn ready_after_first_turn() -> CodexTurnDriver {
    let (mut driver, _) = CodexTurnDriver::start(config(), "first", None).unwrap();
    driver.receive(br#"{"id":1,"result":{}}"#).unwrap();
    driver
        .receive(br#"{"id":2,"result":{"thread":{"id":"thread-private"}}}"#)
        .unwrap();
    driver
        .receive(br#"{"id":3,"result":{"turn":{"id":"turn-1"}}}"#)
        .unwrap();
    driver
        .receive(br#"{"method":"turn/started","params":{"threadId":"thread-private","turn":{"id":"turn-1"}}}"#)
        .unwrap();
    driver
        .receive(br#"{"method":"turn/completed","params":{"threadId":"thread-private","turn":{"id":"turn-1","status":"completed"}}}"#)
        .unwrap();
    driver
}

fn session_states(effects: &[CodexTurnEffect]) -> Vec<bool> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            CodexTurnEffect::SessionState { idle } => Some(*idle),
            _ => None,
        })
        .collect()
}

#[test]
fn the_root_thread_status_reports_whether_the_provider_session_is_running() {
    let mut driver = ready_after_first_turn();
    let status = |driver: &mut CodexTurnDriver, status: &str| {
        let frame = format!(
            r#"{{"method":"thread/status/changed","params":{{"threadId":"thread-private","status":{status}}}}}"#
        );
        session_states(&driver.receive(frame.as_bytes()).unwrap())
    };
    assert_eq!(
        status(&mut driver, r#"{"type":"active","activeFlags":[]}"#),
        [false]
    );
    assert_eq!(
        status(
            &mut driver,
            r#"{"type":"active","activeFlags":["waitingOnApproval"]}"#
        ),
        [false]
    );
    assert_eq!(status(&mut driver, r#"{"type":"idle"}"#), [true]);
    assert_eq!(status(&mut driver, r#"{"type":"systemError"}"#), [true]);
    let child = driver
        .receive(br#"{"method":"thread/status/changed","params":{"threadId":"child","status":{"type":"active","activeFlags":[]}}}"#)
        .unwrap();
    assert!(session_states(&child).is_empty());
}

#[test]
fn a_root_turn_the_client_did_not_start_is_reported_before_its_facts_and_owns_its_completion() {
    let mut driver = ready_after_first_turn();
    let started = driver
        .receive(br#"{"method":"turn/started","params":{"threadId":"thread-private","turn":{"id":"turn-2"}}}"#)
        .unwrap();
    assert_eq!(started.first(), Some(&CodexTurnEffect::ProviderTurnStarted));
    assert!(
        started.contains(&CodexTurnEffect::Fact(PublicWireFact::Event(
            NormalizedProviderEvent::TurnStarted {
                turn_id: "turn-2".into(),
            },
        )))
    );
    let completed = driver
        .receive(br#"{"method":"turn/completed","params":{"threadId":"thread-private","turn":{"id":"turn-2","status":"completed"}}}"#)
        .unwrap();
    assert!(
        completed.contains(&CodexTurnEffect::Fact(PublicWireFact::Event(
            NormalizedProviderEvent::TurnEnded {
                turn_id: "turn-2".into(),
            },
        )))
    );
    assert!(!driver.submit("next", None, &[], None).unwrap().is_empty());
}

#[test]
fn the_client_turn_announcement_is_not_a_provider_turn() {
    let (mut driver, _) = CodexTurnDriver::start(config(), "first", None).unwrap();
    driver.receive(br#"{"id":1,"result":{}}"#).unwrap();
    driver
        .receive(br#"{"id":2,"result":{"thread":{"id":"thread-private"}}}"#)
        .unwrap();
    for frame in [
        &br#"{"method":"turn/started","params":{"threadId":"thread-private","turn":{"id":"turn-1"}}}"#[..],
        br#"{"id":3,"result":{"turn":{"id":"turn-1"}}}"#,
        br#"{"method":"turn/started","params":{"threadId":"thread-private","turn":{"id":"turn-1"}}}"#,
    ] {
        assert!(
            !driver
                .receive(frame)
                .unwrap()
                .contains(&CodexTurnEffect::ProviderTurnStarted)
        );
    }
}

#[test]
fn a_usage_limit_failure_carries_the_reset_time_of_the_exhausted_window() {
    let mut driver = ready_after_first_turn();
    driver
        .receive(br#"{"method":"account/rateLimits/updated","params":{"rateLimits":{"primary":{"usedPercent":40,"resetsAt":1000},"secondary":{"usedPercent":100,"resetsAt":2000}}}}"#)
        .unwrap();
    driver.submit("again", None, &[], None).unwrap();
    driver
        .receive(br#"{"id":4,"result":{"turn":{"id":"turn-2"}}}"#)
        .unwrap();
    let completed = driver
        .receive(br#"{"method":"turn/completed","params":{"threadId":"thread-private","turn":{"id":"turn-2","status":"failed","error":{"message":"limit","codexErrorInfo":"usageLimitExceeded"}}}}"#)
        .unwrap();
    assert!(
        completed.contains(&CodexTurnEffect::Fact(PublicWireFact::Event(
            NormalizedProviderEvent::ProviderFailure {
                classification: gent_types::ProviderFailureClassification::RateLimited,
                message: "Codex usage limit reached.".into(),
                resets_at: Some(2000),
            },
        )))
    );
}
