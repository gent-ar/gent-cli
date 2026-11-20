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

    assert!(driver
        .receive(br#"{"method":"turn/completed","params":{"threadId":"child-thread-1","turn":{"id":"child-turn-1","status":"completed"}}}"#)
        .unwrap()
        .is_empty());

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
    let (mut driver, _) = CodexTurnDriver::start(config(), "hello", None).unwrap();
    driver
        .receive(br#"{"method":"item/completed","params":{"item":{"type":"subAgentActivity","kind":"started","id":"parent-tool-1","agentThreadId":"child-thread-1"}}}"#)
        .unwrap();
    let effects = driver
        .receive(
            serde_json::to_string(&json!({
                "method": "thread/status/changed",
                "params": {"threadId": "child-thread-1", "status": {"type": "active"}}
            }))
            .unwrap()
            .as_bytes(),
        )
        .unwrap();
    assert!(
        effects.contains(&CodexTurnEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::ChildPhase {
                child_id: "child-thread-1".into(),
                phase: WorkPhase::Running,
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
fn child_turn_completion_ignores_statuses_outside_the_codex_schema() {
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
        assert!(!terminal.iter().any(|effect| matches!(
            effect,
            CodexTurnEffect::Fact(PublicWireFact::Event(
                NormalizedProviderEvent::ChildTerminal { .. }
            ))
        )));
        let closed = driver
            .receive(br#"{"method":"thread/closed","params":{"threadId":"child-thread-1"}}"#)
            .unwrap();
        assert!(closed.iter().any(|effect| matches!(
            effect,
            CodexTurnEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::ChildTerminal { child_id, phase }))
                if child_id == "child-thread-1" && *phase == WorkPhase::Interrupted
        )));
    }
}

#[test]
fn a_child_thread_reports_its_work_under_its_spawning_tool_and_never_as_a_root_turn() {
    let (mut driver, _) = CodexTurnDriver::start(config(), "hello", None).unwrap();
    for frame in [
        r#"{"id":1,"result":{}}"#,
        r#"{"id":2,"result":{"thread":{"id":"root-thread"}}}"#,
        r#"{"id":3,"result":{"turn":{"id":"root-turn"}}}"#,
        r#"{"method":"turn/started","params":{"threadId":"root-thread","turn":{"id":"root-turn"}}}"#,
        r#"{"method":"item/started","params":{"threadId":"root-thread","turnId":"root-turn","item":{"type":"collabAgentToolCall","id":"spawn-1","receiverThreadIds":["child-thread"]}}}"#,
    ] {
        driver.receive(frame.as_bytes()).unwrap();
    }
    let facts = |effects: Vec<CodexTurnEffect>| {
        effects
            .into_iter()
            .filter_map(|effect| match effect {
                CodexTurnEffect::Fact(fact) => Some(fact),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let child_turn = facts(
        driver
            .receive(br#"{"method":"turn/started","params":{"threadId":"child-thread","turn":{"id":"child-turn"}}}"#)
            .unwrap(),
    );
    assert_eq!(
        child_turn,
        [PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::ChildPhase {
                child_id: "child-thread".into(),
                phase: WorkPhase::Running,
            }
        )]
    );
    let child_tool = facts(
        driver
            .receive(br#"{"method":"item/started","params":{"threadId":"child-thread","turnId":"child-turn","item":{"type":"commandExecution","id":"exec-1","command":"ls"}}}"#)
            .unwrap(),
    );
    assert!(matches!(
        child_tool.as_slice(),
        [PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity })]
            if activity.tool_use_id == "exec-1"
                && activity.parent_tool_use_id.as_deref() == Some("spawn-1")
    ));
    let child_text = facts(
        driver
            .receive(br#"{"method":"item/agentMessage/delta","params":{"threadId":"child-thread","turnId":"child-turn","itemId":"m","delta":"found it"}}"#)
            .unwrap(),
    );
    assert_eq!(
        child_text,
        [PublicWireFact::Event(
            NormalizedProviderEvent::ToolOutputDelta {
                tool_use_id: "spawn-1".into(),
                text: "found it".into(),
                is_partial: true,
            }
        )]
    );
    let approval = driver
        .receive(br#"{"id":9,"method":"item/commandExecution/requestApproval","params":{"threadId":"child-thread","turnId":"child-turn","itemId":"exec-1"}}"#)
        .unwrap();
    assert!(matches!(
        approval.as_slice(),
        [CodexTurnEffect::ControlRequest(request)]
            if request.child_id.as_deref() == Some("child-thread")
    ));
    let root_approval = driver
        .receive(br#"{"id":10,"method":"item/commandExecution/requestApproval","params":{"threadId":"root-thread","turnId":"root-turn","itemId":"exec-2"}}"#)
        .unwrap();
    assert!(matches!(
        root_approval.as_slice(),
        [CodexTurnEffect::ControlRequest(request)] if request.child_id.is_none()
    ));
}

#[test]
fn a_spawned_child_is_announced_once_when_its_spawn_reports_the_thread() {
    let (mut driver, _) = CodexTurnDriver::start(config(), "hello", None).unwrap();
    for frame in [
        r#"{"id":1,"result":{}}"#,
        r#"{"id":2,"result":{"thread":{"id":"root-thread"}}}"#,
        r#"{"id":3,"result":{"turn":{"id":"root-turn"}}}"#,
    ] {
        driver.receive(frame.as_bytes()).unwrap();
    }
    let announced = |effects: Vec<CodexTurnEffect>| {
        effects
            .into_iter()
            .filter_map(|effect| match effect {
                CodexTurnEffect::Fact(PublicWireFact::Event(
                    NormalizedProviderEvent::ChildStarted {
                        child_id,
                        parent_tool_use_id,
                    },
                )) => Some((child_id, parent_tool_use_id)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let started = driver
        .receive(br#"{"method":"item/started","params":{"threadId":"root-thread","turnId":"root-turn","item":{"type":"collabAgentToolCall","id":"spawn-1","tool":"spawnAgent","receiverThreadIds":[]}}}"#)
        .unwrap();
    assert!(announced(started).is_empty());
    let completed = driver
        .receive(br#"{"method":"item/completed","params":{"threadId":"root-thread","turnId":"root-turn","item":{"type":"collabAgentToolCall","id":"spawn-1","tool":"spawnAgent","status":"completed","receiverThreadIds":["child-thread"]}}}"#)
        .unwrap();
    assert_eq!(
        announced(completed),
        [("child-thread".to_owned(), "spawn-1".to_owned())]
    );
    let waited = driver
        .receive(br#"{"method":"item/started","params":{"threadId":"root-thread","turnId":"root-turn","item":{"type":"collabAgentToolCall","id":"wait-1","tool":"wait","receiverThreadIds":["child-thread"]}}}"#)
        .unwrap();
    assert!(announced(waited).is_empty());
}
