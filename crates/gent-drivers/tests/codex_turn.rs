use gent_drivers::codex_session::{CodexSessionConfig, CodexTurnOptions};
use gent_drivers::codex_turn::{CodexTurnDriver, CodexTurnEffect};
use gent_drivers::public_protocol::PublicWireFact;
use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection,
    GOAL_SCHEMA_VERSION, GoalBinding, GoalProjection, GoalRecord, GoalStatus,
    NormalizedLifecycleSignal, NormalizedProviderEvent, WorkPhase,
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
        mcp_servers: None,
    }
}

fn frames(effects: &[CodexTurnEffect]) -> Vec<Value> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            CodexTurnEffect::Write(frame) => {
                let mut frame: Value = serde_json::from_slice(&frame[..frame.len() - 1]).ok()?;
                assert_eq!(frame["jsonrpc"], json!("2.0"));
                frame.as_object_mut()?.remove("jsonrpc");
                Some(frame)
            }
            CodexTurnEffect::Fact(_)
            | CodexTurnEffect::ControlRequest(_)
            | CodexTurnEffect::Steer(_) => None,
        })
        .collect()
}

fn goal(revision: u64) -> GoalProjection {
    GoalProjection::from_active(&GoalRecord {
        schema_version: GOAL_SCHEMA_VERSION,
        binding: GoalBinding {
            goal_id: "goal-1".into(),
            conversation_id: AgentChatConversationId("conversation-1".into()),
        },
        revision: 1,
        status: GoalStatus::Active,
        reason: gent_types::GoalStatusReason::UserSet,
        objective: format!("Finish the durable task {revision}"),
        note: None,
        time_used_seconds: 0,
        active_since: Some(1),
        tokens_used: 0,
        token_budget: None,
        turns_without_progress: 0,
        accounted_through_ordinal: 0,
        created_at: 1,
        updated_at: 1,
    })
    .unwrap()
}

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
            json!({"id":3,"method":"turn/start","params":{"threadId":"thread-private","input":[{"type":"text","text":"hello"}],"model":"gpt-5.6","effort":"medium","approvalPolicy":"untrusted","sandboxPolicy":{"type":"workspaceWrite","writableRoots":["/work"],"networkAccess":false,"excludeTmpdirEnvVar":false,"excludeSlashTmp":false}}})
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
            json!({"id":4,"method":"turn/start","params":{"threadId":"thread-private","input":[{"type":"text","text":"follow-up"}],"model":"gpt-5.6","effort":"medium","approvalPolicy":"untrusted","sandboxPolicy":{"type":"workspaceWrite","writableRoots":["/work"],"networkAccess":false,"excludeTmpdirEnvVar":false,"excludeSlashTmp":false}}})
        ]
    );
}

#[test]
fn fails_closed_for_server_to_client_requests_without_wedging_the_turn() {
    let (mut driver, _) = CodexTurnDriver::start(config(), "hello", None).unwrap();
    let dynamic_tool = driver
        .receive(br#"{"jsonrpc":"2.0","id":"private-request","method":"item/tool/call","params":{"tool":"private"}}"#)
        .unwrap();
    assert_eq!(
        frames(&dynamic_tool),
        vec![json!({
            "id":"private-request",
            "result":{"success":false,"contentItems":[{
                "type":"inputText",
                "text":"Gent has no registered executor for this experimental Codex dynamic tool."
            }]}
        })]
    );
    let auth = driver
        .receive(br#"{"jsonrpc":"2.0","id":9,"method":"account/chatgptAuthTokens/refresh"}"#)
        .unwrap();
    assert_eq!(frames(&auth)[0]["error"]["code"], json!(-32603));
    let malformed = driver
        .receive(br#"{"jsonrpc":"2.0","id":null,"method":"attestation/generate"}"#)
        .unwrap();
    assert!(matches!(
        malformed.as_slice(),
        [CodexTurnEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::TransportDiagnostic { classification }))]
            if classification == "malformedCodexClientRequest"
    ));
    let next = driver.receive(br#"{"id":1,"result":{}}"#).unwrap();
    assert_eq!(frames(&next)[1]["method"], json!("thread/start"));
}

#[test]
fn answers_every_unrecognized_codex_server_request_with_an_error_so_codex_never_waits() {
    let (mut driver, _) = CodexTurnDriver::start(config(), "hello", None).unwrap();
    let unknown = driver
        .receive(br#"{"jsonrpc":"2.0","id":41,"method":"item/future/requestDecision","params":{"secret":"private"}}"#)
        .unwrap();
    assert_eq!(
        frames(&unknown),
        vec![
            json!({"id":41,"error":{"code":-32601,"message":"Gent does not handle this Codex server request."}})
        ]
    );
    assert!(unknown.iter().any(|effect| matches!(
        effect,
        CodexTurnEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::TransportDiagnostic { classification }))
            if classification == "unsupportedCodexServerRequest"
    )));
    assert!(!format!("{unknown:?}").contains("private"));
    let malformed_approval = driver
        .receive(br#"{"jsonrpc":"2.0","id":"approval-1","method":"item/commandExecution/requestApproval"}"#)
        .unwrap();
    assert_eq!(frames(&malformed_approval)[0]["id"], json!("approval-1"));
    assert_eq!(
        frames(&malformed_approval)[0]["error"]["code"],
        json!(-32601)
    );
    let notification = driver
        .receive(br#"{"jsonrpc":"2.0","method":"item/future/updated","params":{}}"#)
        .unwrap();
    assert!(frames(&notification).is_empty());
    let next = driver.receive(br#"{"id":1,"result":{}}"#).unwrap();
    assert_eq!(frames(&next)[1]["method"], json!("thread/start"));
}

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
