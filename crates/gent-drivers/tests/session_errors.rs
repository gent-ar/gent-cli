use gent_drivers::normalize::normalize;
use gent_drivers::{DriverSession, OutputLimits, SessionEffect, SessionInput, SessionStatus};
use gent_types::{
    NormalizedLifecycleSignal, NormalizedProviderEvent, RootActivity, TurnPhase, WorkPhase,
};
use serde_json::json;

fn raw(value: &str) -> SessionInput {
    SessionInput::RawFrame(value.as_bytes().to_vec())
}

fn diagnostic(classification: &str) -> SessionEffect {
    SessionEffect::Normalized {
        event: NormalizedProviderEvent::TransportDiagnostic {
            classification: classification.into(),
        },
    }
}

fn apply(state: &DriverSession, input: SessionInput) -> gent_drivers::session::SessionTransition {
    state.reduce(input)
}

#[test]
fn session_rejects_invalid_identity_output_and_terminal_ordering() {
    let limits = OutputLimits::new(4, 5);
    let state = DriverSession::new(limits);
    for input in [
        raw("not-json"),
        raw(r#"{"type":"output","text":"ok"}"#),
        raw(r#"{"type":"terminal","reason":"done"}"#),
        raw(r#"{"type":"turn_started"}"#),
    ] {
        assert_eq!(apply(&state, input).effects.len(), 1);
    }
    assert_eq!(
        apply(&state, raw("not-json")).effects,
        vec![diagnostic("malformedProviderFrame")]
    );
    assert_eq!(
        apply(&state, raw(r#"{"type":"output","text":"ok"}"#)).effects,
        vec![diagnostic("outputBeforeSessionId")]
    );
    assert_eq!(
        apply(&state, raw(r#"{"type":"terminal","reason":"done"}"#)).effects,
        vec![diagnostic("terminalBeforeSessionId")]
    );
    assert_eq!(
        apply(&state, raw(r#"{"type":4}"#)).effects,
        vec![diagnostic("malformedProviderFrame")]
    );
    assert_eq!(
        apply(&state, raw(r#"{"type":"session_started"}"#)).effects,
        vec![diagnostic("malformedProviderFrame")]
    );

    let active = apply(
        &state,
        raw(r#"{"type":"session_started","session_id":"one"}"#),
    )
    .state;
    assert_eq!(active.status, SessionStatus::Active);
    assert_eq!(
        apply(
            &active,
            raw(r#"{"type":"session_started","session_id":"one"}"#)
        )
        .effects,
        vec![diagnostic("duplicateSessionId")]
    );
    assert_eq!(
        apply(
            &active,
            raw(r#"{"type":"session_started","session_id":"two"}"#)
        )
        .effects,
        vec![diagnostic("sessionIdChanged")]
    );
    assert_eq!(
        apply(&active, raw(r#"{"type":"output","text":2}"#)).effects,
        vec![diagnostic("malformedProviderFrame")]
    );
    assert_eq!(
        apply(&active, raw(r#"{"type":"output","text":"large"}"#)).effects,
        vec![diagnostic("outputLimitExceeded")]
    );
    assert_eq!(
        apply(&active, raw(r#"{"type":"terminal"}"#)).effects,
        vec![diagnostic("malformedProviderFrame")]
    );
    let output = apply(&active, raw(r#"{"type":"output","text":"four"}"#));
    assert_eq!(output.state.accepted_output_bytes, 4);
    assert_eq!(
        apply(&output.state, raw(r#"{"type":"output","text":"two"}"#)).effects,
        vec![diagnostic("outputLimitExceeded")]
    );

    let terminal = apply(&active, raw(r#"{"type":"terminal","reason":"done"}"#)).state;
    assert_eq!(terminal.status, SessionStatus::Terminal);
    assert_eq!(
        apply(&terminal, raw(r#"{"type":"output","text":"ok"}"#)).effects,
        vec![diagnostic("frameAfterTerminal")]
    );
    assert_eq!(
        apply(&active, SessionInput::RestartRequested).effects,
        vec![diagnostic("restartNotAllowed")]
    );
    assert_eq!(
        apply(&terminal, SessionInput::ProcessExited { code: None }).effects,
        Vec::new()
    );
}

#[test]
fn active_sessions_surface_lifecycle_signals_without_content_effects() {
    let awaiting = DriverSession::new(OutputLimits::new(8, 16));
    let raw_signal = br#"{"type":"root_phase","phase":"waiting_question"}"#;
    assert_eq!(awaiting.lifecycle_signal(raw_signal), None);
    let active = apply(
        &awaiting,
        raw(r#"{"type":"session_started","session_id":"one"}"#),
    )
    .state;
    assert_eq!(
        active.lifecycle_signal(raw_signal),
        Some(NormalizedLifecycleSignal::RootPhase {
            phase: TurnPhase::WaitingQuestion,
        })
    );
    assert!(
        apply(
            &active,
            raw(r#"{"type":"root_phase","phase":"waiting_question"}"#)
        )
        .effects
        .is_empty()
    );
    assert_eq!(
        active.lifecycle_signal(br#"{"type":"root_activity","activity":"generating"}"#),
        Some(NormalizedLifecycleSignal::RootActivity {
            activity: RootActivity::Generating,
        })
    );
    assert_eq!(active.lifecycle_signal(b"not-json"), None);
    assert_eq!(
        active.lifecycle_signal(br#"{"type":"child_phase","child_id":"","phase":"running"}"#),
        None
    );
}

#[test]
fn exit_recovery_only_restarts_before_session_identity() {
    let initial = DriverSession::new(OutputLimits::new(8, 16));
    let restartable = apply(&initial, SessionInput::ProcessExited { code: None });
    assert_eq!(restartable.state.status, SessionStatus::Restartable);
    assert_eq!(
        restartable.effects,
        vec![diagnostic("providerExitedBeforeSessionId")]
    );
    let retried = apply(&restartable.state, SessionInput::RestartRequested);
    assert_eq!(retried.state.attempt, 2);
    assert_eq!(
        retried.effects,
        vec![SessionEffect::StartAttempt { attempt: 2 }]
    );
    let active = apply(
        &retried.state,
        raw(r#"{"type":"session_started","session_id":"two"}"#),
    )
    .state;
    assert_eq!(
        apply(&active, SessionInput::ProcessExited { code: Some(7) }).effects,
        vec![SessionEffect::Terminal {
            reason: "providerExited:7".into()
        }]
    );
    assert_eq!(
        apply(
            &restartable.state,
            raw(r#"{"type":"session_started","session_id":"late"}"#)
        )
        .effects,
        vec![diagnostic("unexpectedSessionId")]
    );
}

#[test]
fn lifecycle_frames_require_complete_identifiers() {
    let initial = DriverSession::new(OutputLimits::new(8, 16));
    let active = apply(
        &initial,
        raw(r#"{"type":"session_started","session_id":"two"}"#),
    )
    .state;
    for frame in [
        r#"{"type":"turn_started","turn_id":"turn"}"#,
        r#"{"type":"turn_ended","turn_id":"turn"}"#,
        r#"{"type":"child_started","child_id":"child","parent_tool_use_id":"tool"}"#,
        r#"{"type":"child_terminal","child_id":"child","phase":"done"}"#,
        r#"{"type":"command_terminal","command_id":"command","phase":"done"}"#,
        r#"{"type":"decision_settled","decision_id":"decision"}"#,
    ] {
        assert!(matches!(
            apply(&active, raw(frame)).effects.as_slice(),
            [SessionEffect::Normalized { .. }]
        ));
    }
    assert_eq!(
        apply(&active, raw(r#"{"type":"future/frame"}"#)).effects,
        vec![diagnostic("unknownProviderFrame")]
    );
    for frame in [
        r#"{"type":"turn_started"}"#,
        r#"{"type":"child_started","child_id":"child"}"#,
        r#"{"type":"child_terminal","child_id":"child"}"#,
        r#"{"type":"command_terminal","command_id":"command"}"#,
        r#"{"type":"decision_settled"}"#,
    ] {
        assert_eq!(
            apply(&active, raw(frame)).effects,
            vec![diagnostic("malformedProviderFrame")]
        );
    }
}

#[test]
fn normalization_preserves_known_lifecycle_events_and_safe_defaults() {
    for (frame, expected) in [
        (
            json!({"type":"turn_started","turn_id":"turn"}),
            NormalizedProviderEvent::TurnStarted {
                turn_id: "turn".into(),
            },
        ),
        (
            json!({"type":"turn_ended","turn_id":"turn"}),
            NormalizedProviderEvent::TurnEnded {
                turn_id: "turn".into(),
            },
        ),
        (
            json!({"type":"child_started","child_id":"child","parent_tool_use_id":"tool"}),
            NormalizedProviderEvent::ChildStarted {
                child_id: "child".into(),
                parent_tool_use_id: "tool".into(),
            },
        ),
        (
            json!({"type":"child_terminal","child_id":"child","phase":"waiting_permission"}),
            NormalizedProviderEvent::ChildTerminal {
                child_id: "child".into(),
                phase: WorkPhase::WaitingPermission,
            },
        ),
        (
            json!({"type":"command_terminal","command_id":"command","phase":"failed"}),
            NormalizedProviderEvent::CommandTerminal {
                command_id: "command".into(),
                phase: WorkPhase::Failed,
            },
        ),
        (
            json!({"type":"decision_settled","decision_id":"decision"}),
            NormalizedProviderEvent::DecisionSettled {
                decision_id: "decision".into(),
            },
        ),
    ] {
        assert_eq!(normalize(&frame), expected);
    }
    assert_eq!(
        normalize(&json!({"type":"child_terminal","child_id":"child","phase":"pending"})),
        NormalizedProviderEvent::ChildTerminal {
            child_id: "child".into(),
            phase: WorkPhase::Pending
        }
    );
    assert_eq!(
        normalize(&json!({"type":"child_terminal","child_id":"child","phase":"interrupted"})),
        NormalizedProviderEvent::ChildTerminal {
            child_id: "child".into(),
            phase: WorkPhase::Interrupted
        }
    );
    assert_eq!(
        normalize(&json!({"type":"child_terminal","child_id":"child","phase":"future"})),
        NormalizedProviderEvent::ChildTerminal {
            child_id: "child".into(),
            phase: WorkPhase::Running
        }
    );
    assert_eq!(
        normalize(&json!({})),
        NormalizedProviderEvent::TransportDiagnostic {
            classification: "unknownProviderFrame".into()
        }
    );
}
