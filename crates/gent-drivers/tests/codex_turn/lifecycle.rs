use super::*;

#[test]
fn codex_receives_the_same_gent_owned_goal_for_each_turn() {
    let initial_goal = goal(3);
    let (mut driver, _) = CodexTurnDriver::start(config(), "first", Some(&initial_goal)).unwrap();
    driver.receive(br#"{"id":1,"result":{}}"#).unwrap();
    let effects = driver
        .receive(br#"{"id":2,"result":{"thread":{"id":"thread-private"}}}"#)
        .unwrap();
    let initial_frames = frames(&effects);
    let initial = initial_frames[0]["params"]["input"][0]["text"]
        .as_str()
        .unwrap();
    assert!(initial.contains("\"goalId\":\"goal-1\""));

    driver
        .receive(br#"{"method":"turn/started","params":{"threadId":"thread-private","turn":{"id":"turn-1"}}}"#)
        .unwrap();
    driver
        .receive(br#"{"id":3,"result":{"turn":{"id":"turn-1"}}}"#)
        .unwrap();
    driver
        .receive(br#"{"method":"turn/completed","params":{"threadId":"thread-private","turn":{"id":"turn-1"}}}"#)
        .unwrap();
    let later_goal = goal(4);
    let later_frames = frames(
        &driver
            .submit("continue", Some(&later_goal), &[], None)
            .unwrap(),
    );
    let later = later_frames[0]["params"]["input"][0]["text"]
        .as_str()
        .unwrap();
    assert!(later.contains("Finish the durable task 4"));
    assert!(later.ends_with("User prompt:\ncontinue"));
}

#[test]
fn empty_turn_completion_uses_the_owned_live_turn_identity() {
    let (mut driver, _) = CodexTurnDriver::start(config(), "hello", None).unwrap();
    driver.receive(br#"{"id":1,"result":{}}"#).unwrap();
    driver
        .receive(br#"{"id":2,"result":{"thread":{"id":"thread-private"}}}"#)
        .unwrap();
    driver
        .receive(br#"{"method":"turn/started","params":{"threadId":"thread-private","turn":{"id":"turn-1"}}}"#)
        .unwrap();
    driver
        .receive(br#"{"id":3,"result":{"turn":{"id":"turn-1"}}}"#)
        .unwrap();
    let effects = driver
        .receive(br#"{"method":"turn/completed","params":{}}"#)
        .unwrap();
    assert!(
        effects.contains(&CodexTurnEffect::Fact(PublicWireFact::Event(
            NormalizedProviderEvent::TurnEnded {
                turn_id: "turn-1".into(),
            },
        )))
    );
    assert!(!effects.iter().any(|effect| matches!(
        effect,
        CodexTurnEffect::Fact(PublicWireFact::Event(
            NormalizedProviderEvent::TransportDiagnostic { .. }
        ))
    )));
}

#[test]
fn reports_one_model_failure_for_a_failed_turn() {
    let (mut driver, _) = CodexTurnDriver::start(config(), "hello", None).unwrap();
    driver.receive(br#"{"id":1,"result":{}}"#).unwrap();
    driver
        .receive(br#"{"id":2,"result":{"thread":{"id":"thread-private"}}}"#)
        .unwrap();
    driver
        .receive(br#"{"method":"turn/started","params":{"threadId":"thread-private","turn":{"id":"turn-1"}}}"#)
        .unwrap();
    let error = driver
        .receive(br#"{"method":"error","params":{"error":{"message":"The selected model is not supported."}}}"#)
        .unwrap();
    assert!(error.contains(&CodexTurnEffect::Fact(PublicWireFact::Event(
        NormalizedProviderEvent::ProviderFailure {
            classification: gent_types::ProviderFailureClassification::Provider,
            message: "The selected Codex model is unavailable.".into(),
        },
    ))));
    let completed = driver
        .receive(br#"{"method":"turn/completed","params":{"threadId":"thread-private","turn":{"id":"turn-1","status":"failed","error":{"message":"The selected model is not supported."}}}}"#)
        .unwrap();
    assert!(!completed.iter().any(|effect| matches!(
        effect,
        CodexTurnEffect::Fact(PublicWireFact::Event(
            NormalizedProviderEvent::ProviderFailure { .. }
        ))
    )));
}

#[test]
fn command_completion_supplies_canonical_output_only_when_no_delta_arrived() {
    let (mut driver, _) = CodexTurnDriver::start(config(), "hello", None).unwrap();
    driver.receive(br#"{"id":1,"result":{}}"#).unwrap();
    driver
        .receive(br#"{"id":2,"result":{"thread":{"id":"thread-private"}}}"#)
        .unwrap();
    driver
        .receive(br#"{"method":"turn/started","params":{"threadId":"thread-private","turn":{"id":"turn-1"}}}"#)
        .unwrap();
    driver
        .receive(br#"{"id":3,"result":{"turn":{"id":"turn-1"}}}"#)
        .unwrap();

    let fallback = driver
        .receive(br#"{"method":"item/completed","params":{"item":{"type":"commandExecution","id":"cmd-1","aggregatedOutput":"canonical output"}}}"#)
        .unwrap();
    assert!(fallback.iter().any(|effect| matches!(
        effect,
        CodexTurnEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::ToolOutputDelta {
            tool_use_id, text, is_partial
        })) if tool_use_id == "cmd-1" && text == "canonical output" && !*is_partial
    )));

    let repeated = driver
        .receive(br#"{"method":"item/completed","params":{"item":{"type":"commandExecution","id":"cmd-1","aggregatedOutput":"canonical output"}}}"#)
        .unwrap();
    assert!(!repeated.iter().any(|effect| matches!(
        effect,
        CodexTurnEffect::Fact(PublicWireFact::Event(
            NormalizedProviderEvent::ToolOutputDelta { .. }
        ))
    )));

    let (mut streamed, _) = CodexTurnDriver::start(config(), "hello", None).unwrap();
    streamed.receive(br#"{"id":1,"result":{}}"#).unwrap();
    streamed
        .receive(br#"{"id":2,"result":{"thread":{"id":"thread-private"}}}"#)
        .unwrap();
    streamed
        .receive(br#"{"method":"turn/started","params":{"threadId":"thread-private","turn":{"id":"turn-1"}}}"#)
        .unwrap();
    streamed
        .receive(br#"{"id":3,"result":{"turn":{"id":"turn-1"}}}"#)
        .unwrap();
    streamed
        .receive(br#"{"method":"item/commandExecution/outputDelta","params":{"itemId":"cmd-2","delta":"streamed"}}"#)
        .unwrap();
    let no_duplicate = streamed
        .receive(br#"{"method":"item/completed","params":{"item":{"type":"commandExecution","id":"cmd-2","aggregatedOutput":"streamed"}}}"#)
        .unwrap();
    assert!(!no_duplicate.iter().any(|effect| matches!(
        effect,
        CodexTurnEffect::Fact(PublicWireFact::Event(
            NormalizedProviderEvent::ToolOutputDelta { .. }
        ))
    )));
}

#[test]
fn handshakes_then_starts_the_exact_one_prompt_without_exporting_native_ids() {
    let (mut driver, initial) = CodexTurnDriver::start(config(), "hello", None).unwrap();
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
    assert!(
        next.contains(&CodexTurnEffect::Fact(PublicWireFact::SessionStarted {
            provider_session_id: "thread-private".into(),
        }))
    );
    assert_eq!(
        frames(&next),
        vec![
            json!({"id":3,"method":"turn/start","params":{"threadId":"thread-private","input":[{"type":"text","text":"hello"}],"model":"gpt-5.6","effort":"medium","summary":"auto","approvalPolicy":"untrusted","sandboxPolicy":{"type":"workspaceWrite","writableRoots":["/work"],"networkAccess":false,"excludeTmpdirEnvVar":false,"excludeSlashTmp":false}}})
        ]
    );
}

#[test]
fn records_only_the_correlated_thread_identity_when_thread_started_arrives_early() {
    let (mut driver, _) = CodexTurnDriver::start(config(), "hello", None).unwrap();
    driver.receive(br#"{"id":1,"result":{}}"#).unwrap();
    let notification = driver
        .receive(
            br#"{"method":"thread/started","params":{"thread":{"id":"untrusted-notification"}}}"#,
        )
        .unwrap();
    assert!(!notification.iter().any(|effect| matches!(
        effect,
        CodexTurnEffect::Fact(PublicWireFact::SessionStarted { .. })
    )));
    let response = driver
        .receive(br#"{"id":2,"result":{"thread":{"id":"thread-private"}}}"#)
        .unwrap();
    assert_eq!(
        response
            .iter()
            .filter(|effect| matches!(
                effect,
                CodexTurnEffect::Fact(PublicWireFact::SessionStarted { .. })
            ))
            .count(),
        1
    );
    assert!(
        response.contains(&CodexTurnEffect::Fact(PublicWireFact::SessionStarted {
            provider_session_id: "thread-private".into(),
        }))
    );
}

#[test]
fn normalizes_notifications_but_discards_malformed_notification_state() {
    let (mut driver, _) = CodexTurnDriver::start(config(), "hello", None).unwrap();
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
    let (mut driver, _) = CodexTurnDriver::start(config(), "hello", None).unwrap();
    assert!(matches!(
        driver.receive(b"not-json").unwrap().as_slice(),
        [CodexTurnEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::TransportDiagnostic { classification }))]
            if classification == "malformedCodexFrame"
    ));
    assert!(
        driver
            .receive(&vec![b'x'; gent_drivers::MAX_PROVIDER_FRAME_BYTES + 1])
            .is_err()
    );
    assert!(driver.receive(&vec![b'x'; 65_537]).is_ok());
}

#[test]
fn reuses_the_ready_native_thread_for_a_later_prompt() {
    let (mut driver, _) = CodexTurnDriver::start(config(), "first", None).unwrap();
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
        frames(&driver.submit("follow-up", None, &[], None).unwrap()),
        vec![
            json!({"id":4,"method":"turn/start","params":{"threadId":"thread-private","input":[{"type":"text","text":"follow-up"}],"model":"gpt-5.6","effort":"medium","summary":"auto","approvalPolicy":"untrusted","sandboxPolicy":{"type":"workspaceWrite","writableRoots":["/work"],"networkAccess":false,"excludeTmpdirEnvVar":false,"excludeSlashTmp":false}}})
        ]
    );
}
