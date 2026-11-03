use super::*;

#[test]
fn settles_only_a_known_child_thread_on_explicit_failure_or_cancellation() {
    let (mut driver, _) = CodexTurnDriver::start(config(), "hello", None).unwrap();
    let launch = driver
        .receive(br#"{"method":"item/completed","params":{"item":{"type":"subAgentActivity","kind":"started","id":"parent-tool-1","agentThreadId":"child-thread-1"}}}"#)
        .unwrap();
    assert!(
        launch.contains(&CodexTurnEffect::Fact(PublicWireFact::Event(
            NormalizedProviderEvent::ChildStarted {
                child_id: "child-thread-1".into(),
                parent_tool_use_id: "parent-tool-1".into(),
            }
        )))
    );

    let root_idle = driver
        .receive(br#"{"method":"thread/status/changed","params":{"threadId":"root-thread","status":{"type":"systemError"}}}"#)
        .unwrap();
    assert!(root_idle.is_empty());

    let idle = driver
        .receive(br#"{"method":"thread/status/changed","params":{"threadId":"child-thread-1","status":{"type":"idle"}}}"#)
        .unwrap();
    assert!(idle.is_empty());

    let terminal = driver
        .receive(br#"{"method":"thread/status/changed","params":{"threadId":"child-thread-1","status":{"type":"systemError"}}}"#)
        .unwrap();
    assert_eq!(
        terminal,
        vec![CodexTurnEffect::Fact(PublicWireFact::Event(
            NormalizedProviderEvent::ChildTerminal {
                child_id: "child-thread-1".into(),
                phase: WorkPhase::Failed,
            }
        ))]
    );
    let duplicate = driver
        .receive(br#"{"method":"thread/status/changed","params":{"threadId":"child-thread-1","status":{"type":"cancelled"}}}"#)
        .unwrap();
    assert!(duplicate.is_empty());
}

#[test]
fn settles_a_known_child_only_when_its_own_turn_explicitly_completes() {
    let (mut driver, _) = CodexTurnDriver::start(config(), "hello", None).unwrap();
    driver
        .receive(br#"{"method":"item/completed","params":{"item":{"type":"subAgentActivity","kind":"started","id":"parent-tool-1","agentThreadId":"child-thread-1"}}}"#)
        .unwrap();

    let terminal = driver
        .receive(br#"{"method":"turn/completed","params":{"threadId":"child-thread-1","turn":{"id":"child-turn-1","status":"completed"}}}"#)
        .unwrap();
    assert_eq!(
        terminal,
        vec![CodexTurnEffect::Fact(PublicWireFact::Event(
            NormalizedProviderEvent::ChildTerminal {
                child_id: "child-thread-1".into(),
                phase: WorkPhase::Done,
            }
        ))]
    );

    // Repeated provider terminal evidence cannot create a second settlement.
    assert!(driver
        .receive(br#"{"method":"turn/completed","params":{"threadId":"child-thread-1","turn":{"id":"child-turn-1","status":"completed"}}}"#)
        .unwrap()
        .is_empty());

    // An uncorrelated root completion remains a root fact, not a child terminal.
    let root = driver
        .receive(br#"{"method":"turn/completed","params":{"threadId":"root-thread","turn":{"id":"root-turn-1","status":"completed"}}}"#)
        .unwrap();
    assert!(root.contains(&CodexTurnEffect::Fact(PublicWireFact::Event(
        NormalizedProviderEvent::TurnEnded {
            turn_id: "root-turn-1".into(),
        }
    ))));
    assert!(!root.iter().any(|effect| matches!(
        effect,
        CodexTurnEffect::Fact(PublicWireFact::Event(
            NormalizedProviderEvent::ChildTerminal { .. }
        ))
    )));
}

#[test]
fn preserves_live_child_statuses_without_settling_the_child() {
    for (status, phase) in [
        ("pending", WorkPhase::Pending),
        ("queued", WorkPhase::Pending),
        ("working", WorkPhase::Running),
        ("running", WorkPhase::Running),
    ] {
        let (mut driver, _) = CodexTurnDriver::start(config(), "hello", None).unwrap();
        driver
            .receive(br#"{"method":"item/completed","params":{"item":{"type":"subAgentActivity","kind":"started","id":"parent-tool-1","agentThreadId":"child-thread-1"}}}"#)
            .unwrap();
        let effects = driver
            .receive(
                serde_json::to_string(&json!({
                    "method": "thread/status/changed",
                    "params": {"threadId": "child-thread-1", "status": {"type": status}}
                }))
                .unwrap()
                .as_bytes(),
            )
            .unwrap();
        assert!(
            effects.contains(&CodexTurnEffect::Fact(PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::ChildPhase {
                    child_id: "child-thread-1".into(),
                    phase,
                },
            )))
        );
        assert!(!effects.iter().any(|effect| matches!(
            effect,
            CodexTurnEffect::Fact(PublicWireFact::Event(
                NormalizedProviderEvent::ChildTerminal { .. }
            ))
        )));
    }
}

#[test]
fn settles_a_known_child_on_failed_or_interrupted_turn_completion() {
    for (status, phase) in [
        ("failed", WorkPhase::Failed),
        ("interrupted", WorkPhase::Interrupted),
    ] {
        let (mut driver, _) = CodexTurnDriver::start(config(), "hello", None).unwrap();
        driver
            .receive(br#"{"method":"item/completed","params":{"item":{"type":"subAgentActivity","kind":"started","id":"parent-tool-1","agentThreadId":"child-thread-1"}}}"#)
            .unwrap();
        let terminal = driver
            .receive(
                serde_json::to_string(&json!({
                    "method": "turn/completed",
                    "params": {"threadId": "child-thread-1", "turn": {"id": "child-turn-1", "status": status}}
                }))
                .unwrap()
                .as_bytes(),
            )
            .unwrap();
        assert_eq!(
            terminal,
            vec![CodexTurnEffect::Fact(PublicWireFact::Event(
                NormalizedProviderEvent::ChildTerminal {
                    child_id: "child-thread-1".into(),
                    phase,
                },
            ))]
        );
    }
}

#[test]
fn child_turn_completion_accepts_native_interrupt_status_aliases() {
    for status in [
        "cancelled",
        "canceled",
        "aborted",
        "timedOut",
        "timed_out",
        "timeout",
    ] {
        let (mut driver, _) = CodexTurnDriver::start(config(), "hello", None).unwrap();
        driver
            .receive(br#"{"method":"item/completed","params":{"item":{"type":"subAgentActivity","kind":"started","id":"parent-tool-1","agentThreadId":"child-thread-1"}}}"#)
            .unwrap();
        let terminal = driver
            .receive(
                serde_json::to_string(&json!({
                    "method": "turn/completed",
                    "params": {"threadId": "child-thread-1", "turn": {"id": "child-turn-1", "status": status}}
                }))
                .unwrap()
                .as_bytes(),
            )
            .unwrap();
        assert!(terminal.iter().any(|effect| matches!(
            effect,
            CodexTurnEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::ChildTerminal { child_id, phase }))
                if child_id == "child-thread-1" && *phase == WorkPhase::Interrupted
        )));
    }
}
