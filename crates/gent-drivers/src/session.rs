//! Pure public-provider session reducer. The process owner performs its effects.

use gent_types::NormalizedProviderEvent;
use serde::Serialize;
use serde_json::Value;

use crate::normalize::normalize;

/// Hard bounds for output accepted from a single provider session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutputLimits {
    pub max_frame_bytes: usize,
    pub max_total_bytes: usize,
}

impl OutputLimits {
    /// Builds limits used to reject oversized frames and cumulative output.
    #[must_use]
    pub const fn new(max_frame_bytes: usize, max_total_bytes: usize) -> Self {
        Self {
            max_frame_bytes,
            max_total_bytes,
        }
    }

    fn accepts(self, current: usize, text: &str) -> bool {
        text.len() <= self.max_frame_bytes
            && current.saturating_add(text.len()) <= self.max_total_bytes
    }
}

/// Lifecycle state with no process, timer, or persistence ownership.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionStatus {
    AwaitingSessionId,
    Restartable,
    Active,
    Terminal,
}

/// Complete reducer state for one public-provider launch attempt.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverSession {
    pub attempt: u32,
    pub session_id: Option<String>,
    pub status: SessionStatus,
    pub accepted_output_bytes: usize,
    #[serde(skip)]
    limits: OutputLimits,
}

impl DriverSession {
    #[must_use]
    pub const fn new(limits: OutputLimits) -> Self {
        Self {
            attempt: 1,
            session_id: None,
            status: SessionStatus::AwaitingSessionId,
            accepted_output_bytes: 0,
            limits,
        }
    }

    /// Applies one observed fact and returns only effects for the owning edge.
    #[must_use]
    pub fn reduce(&self, input: SessionInput) -> SessionTransition {
        match input {
            SessionInput::RawFrame(raw) => self.reduce_raw_frame(&raw),
            SessionInput::ProcessExited { code } => self.reduce_exit(code),
            SessionInput::RestartRequested => self.reduce_restart(),
        }
    }

    fn reduce_raw_frame(&self, raw: &[u8]) -> SessionTransition {
        let Ok(frame) = serde_json::from_slice::<Value>(raw) else {
            return self.diagnostic("malformedProviderFrame");
        };
        if self.status == SessionStatus::Terminal {
            return self.diagnostic("frameAfterTerminal");
        }
        let Some(kind) = frame.get("type").and_then(Value::as_str) else {
            return self.diagnostic("malformedProviderFrame");
        };
        match kind {
            "session_started" => self.reduce_session_started(&frame),
            "terminal" => self.reduce_terminal(&frame),
            "output" => self.reduce_output(&frame),
            _ if valid_lifecycle_frame(kind, &frame) => self.emit(normalize(&frame)),
            _ if known_lifecycle_frame(kind) => self.diagnostic("malformedProviderFrame"),
            _ => self.emit(normalize(&frame)),
        }
    }

    fn reduce_session_started(&self, frame: &Value) -> SessionTransition {
        let Some(session_id) = non_empty(frame, "session_id") else {
            return self.diagnostic("malformedProviderFrame");
        };
        match self.status {
            SessionStatus::AwaitingSessionId => SessionTransition::new(
                Self {
                    session_id: Some(session_id.into()),
                    status: SessionStatus::Active,
                    ..self.clone()
                },
                Vec::new(),
            ),
            SessionStatus::Active if self.session_id.as_deref() == Some(session_id) => {
                self.diagnostic("duplicateSessionId")
            }
            SessionStatus::Active => self.diagnostic("sessionIdChanged"),
            SessionStatus::Restartable | SessionStatus::Terminal => {
                self.diagnostic("unexpectedSessionId")
            }
        }
    }

    fn reduce_terminal(&self, frame: &Value) -> SessionTransition {
        let Some(reason) = non_empty(frame, "reason") else {
            return self.diagnostic("malformedProviderFrame");
        };
        if self.session_id.is_none() {
            return self.diagnostic("terminalBeforeSessionId");
        }
        SessionTransition::new(
            Self {
                status: SessionStatus::Terminal,
                ..self.clone()
            },
            vec![SessionEffect::Terminal {
                reason: reason.into(),
            }],
        )
    }

    fn reduce_output(&self, frame: &Value) -> SessionTransition {
        let Some(text) = frame.get("text").and_then(Value::as_str) else {
            return self.diagnostic("malformedProviderFrame");
        };
        if self.session_id.is_none() {
            return self.diagnostic("outputBeforeSessionId");
        }
        if !self.limits.accepts(self.accepted_output_bytes, text) {
            return self.diagnostic("outputLimitExceeded");
        }
        SessionTransition::new(
            Self {
                accepted_output_bytes: self.accepted_output_bytes + text.len(),
                ..self.clone()
            },
            vec![SessionEffect::Normalized {
                event: normalize(frame),
            }],
        )
    }

    fn reduce_exit(&self, code: Option<i32>) -> SessionTransition {
        match self.status {
            SessionStatus::AwaitingSessionId => SessionTransition::new(
                Self {
                    status: SessionStatus::Restartable,
                    ..self.clone()
                },
                vec![diagnostic("providerExitedBeforeSessionId")],
            ),
            SessionStatus::Active => SessionTransition::new(
                Self {
                    status: SessionStatus::Terminal,
                    ..self.clone()
                },
                vec![SessionEffect::Terminal {
                    reason: exit_reason(code),
                }],
            ),
            SessionStatus::Restartable | SessionStatus::Terminal => {
                SessionTransition::new(self.clone(), Vec::new())
            }
        }
    }

    fn reduce_restart(&self) -> SessionTransition {
        if self.status != SessionStatus::Restartable {
            return self.diagnostic("restartNotAllowed");
        }
        SessionTransition::new(
            Self {
                attempt: self.attempt.saturating_add(1),
                session_id: None,
                status: SessionStatus::AwaitingSessionId,
                accepted_output_bytes: 0,
                ..self.clone()
            },
            vec![SessionEffect::StartAttempt {
                attempt: self.attempt.saturating_add(1),
            }],
        )
    }

    fn diagnostic(&self, classification: &str) -> SessionTransition {
        self.emit(NormalizedProviderEvent::TransportDiagnostic {
            classification: classification.into(),
        })
    }

    fn emit(&self, event: NormalizedProviderEvent) -> SessionTransition {
        SessionTransition::new(self.clone(), vec![SessionEffect::Normalized { event }])
    }
}

/// An observed input. Raw frames remain bytes until the reducer validates them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionInput {
    RawFrame(Vec<u8>),
    ProcessExited { code: Option<i32> },
    RestartRequested,
}

/// Typed work for the process/persistence edge after reducing an input.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SessionEffect {
    Normalized { event: NormalizedProviderEvent },
    Terminal { reason: String },
    StartAttempt { attempt: u32 },
}

/// A state/effect pair, making each input independently testable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionTransition {
    pub state: DriverSession,
    pub effects: Vec<SessionEffect>,
}

impl SessionTransition {
    const fn new(state: DriverSession, effects: Vec<SessionEffect>) -> Self {
        Self { state, effects }
    }
}

fn non_empty<'a>(frame: &'a Value, field: &str) -> Option<&'a str> {
    frame.get(field)?.as_str().filter(|value| !value.is_empty())
}

fn known_lifecycle_frame(kind: &str) -> bool {
    matches!(
        kind,
        "turn_started"
            | "turn_ended"
            | "child_started"
            | "child_terminal"
            | "command_terminal"
            | "decision_settled"
    )
}

fn valid_lifecycle_frame(kind: &str, frame: &Value) -> bool {
    match kind {
        "turn_started" | "turn_ended" => non_empty(frame, "turn_id").is_some(),
        "child_started" => {
            non_empty(frame, "child_id").is_some()
                && non_empty(frame, "parent_tool_use_id").is_some()
        }
        "child_terminal" => {
            non_empty(frame, "child_id").is_some() && non_empty(frame, "phase").is_some()
        }
        "command_terminal" => {
            non_empty(frame, "command_id").is_some() && non_empty(frame, "phase").is_some()
        }
        "decision_settled" => non_empty(frame, "decision_id").is_some(),
        _ => false,
    }
}

fn diagnostic(classification: &str) -> SessionEffect {
    SessionEffect::Normalized {
        event: NormalizedProviderEvent::TransportDiagnostic {
            classification: classification.into(),
        },
    }
}

fn exit_reason(code: Option<i32>) -> String {
    match code {
        Some(code) => format!("providerExited:{code}"),
        None => "providerExited:unknown".into(),
    }
}
