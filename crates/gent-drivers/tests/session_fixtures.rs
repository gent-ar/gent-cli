use gent_drivers::{DriverSession, OutputLimits, SessionEffect, SessionInput, SessionStatus};
use serde_json::Value;

const FIXTURES: &str = include_str!("../fixtures/session-reducer.json");

#[test]
fn public_session_fixtures_cover_recovery_and_normalization() {
    let fixtures: Vec<Value> = serde_json::from_str(FIXTURES).unwrap();
    for fixture in fixtures {
        assert_fixture(&fixture);
    }
}

#[test]
fn output_is_rejected_at_the_bound_without_unbounded_accumulation() {
    let mut state = DriverSession::new(OutputLimits::new(4, 5));
    state = apply(
        &state,
        json_input(r#"{"type":"session_started","session_id":"s"}"#),
    )
    .state;
    let first = apply(&state, json_input(r#"{"type":"output","text":"four"}"#));
    assert_eq!(first.state.accepted_output_bytes, 4);
    let limited = apply(
        &first.state,
        json_input(r#"{"type":"output","text":"two"}"#),
    );
    assert_eq!(limited.state.accepted_output_bytes, 4);
    assert_eq!(
        limited.effects,
        vec![diagnostic("outputLimitExceeded")],
        "the reducer retains no unbounded provider output"
    );
}

fn assert_fixture(fixture: &Value) {
    let mut state = DriverSession::new(OutputLimits::new(1024, 4096));
    let mut effects = Vec::new();
    for input in fixture["inputs"].as_array().unwrap() {
        let transition = apply(&state, fixture_input(input));
        state = transition.state;
        effects.extend(transition.effects);
    }
    assert_eq!(
        status_name(state.status),
        fixture["status"].as_str().unwrap()
    );
    let attempt = u32::try_from(fixture["attempt"].as_u64().unwrap()).unwrap();
    assert_eq!(state.attempt, attempt);
    assert_eq!(state.session_id.as_deref(), fixture["sessionId"].as_str());
    assert_eq!(serde_json::to_value(effects).unwrap(), fixture["effects"]);
}

fn fixture_input(value: &Value) -> SessionInput {
    if value["restart"].as_bool() == Some(true) {
        SessionInput::RestartRequested
    } else if let Some(code) = value.get("exit") {
        SessionInput::ProcessExited {
            code: code.as_i64().map(|number| i32::try_from(number).unwrap()),
        }
    } else if let Some(raw) = value["raw"].as_str() {
        SessionInput::RawFrame(raw.as_bytes().to_vec())
    } else {
        SessionInput::RawFrame(serde_json::to_vec(&value["frame"]).unwrap())
    }
}

fn json_input(frame: &str) -> SessionInput {
    SessionInput::RawFrame(frame.as_bytes().to_vec())
}

fn apply(state: &DriverSession, input: SessionInput) -> gent_drivers::session::SessionTransition {
    state.reduce(input)
}

fn diagnostic(classification: &str) -> SessionEffect {
    SessionEffect::Normalized {
        event: gent_types::NormalizedProviderEvent::TransportDiagnostic {
            classification: classification.into(),
        },
    }
}

fn status_name(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::AwaitingSessionId => "awaitingSessionId",
        SessionStatus::Restartable => "restartable",
        SessionStatus::Active => "active",
        SessionStatus::Terminal => "terminal",
    }
}
