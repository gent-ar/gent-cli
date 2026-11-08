use super::ClaudeRunnerEffect;
use crate::public_protocol::PublicWireFact;
use gent_types::{NormalizedLifecycleSignal, NormalizedProviderEvent};
use serde_json::Value;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::time::{Duration, Instant};

pub(super) const TRANSIENT_RETRY_STEP: Duration = Duration::from_secs(3);
const MAX_TRANSIENT_RETRIES: u8 = 3;

#[derive(Debug)]
enum Pending {
    Idle,
    AwaitingResult { resent: bool },
    Due(Instant),
}

#[derive(Debug)]
pub(super) struct TransientRetry {
    step: Duration,
    input: Option<Vec<u8>>,
    worked: bool,
    attempts: u8,
    pending: Pending,
}

impl TransientRetry {
    pub(super) fn new(step: Duration, input: Option<&[u8]>) -> Self {
        Self {
            step,
            input: input.map(<[u8]>::to_vec),
            worked: false,
            attempts: 0,
            pending: Pending::Idle,
        }
    }

    pub(super) fn sent(&mut self, input: &[u8]) {
        *self = Self::new(self.step, Some(input));
    }

    pub(super) fn intercept(&mut self, frame: &Value) -> Option<Vec<ClaudeRunnerEffect>> {
        if let Pending::AwaitingResult { resent } = self.pending
            && frame_type(frame) == Some("result")
        {
            self.pending = if resent {
                Pending::Idle
            } else {
                Pending::Due(Instant::now() + backoff(self.step, self.attempts))
            };
            return Some(Vec::new());
        }
        if self.worked
            || self.attempts >= MAX_TRANSIENT_RETRIES
            || self.input.is_none()
            || !matches!(self.pending, Pending::Idle)
            || !transient_failure(frame)
        {
            return None;
        }
        self.attempts += 1;
        self.pending = Pending::AwaitingResult { resent: false };
        Some(vec![ClaudeRunnerEffect::Fact(PublicWireFact::Event(
            NormalizedProviderEvent::TransportDiagnostic {
                classification: gent_types::PROVIDER_TRANSIENT_RETRY_DIAGNOSTIC.into(),
            },
        ))])
    }

    pub(super) fn observe(&mut self, effects: &[ClaudeRunnerEffect]) {
        self.worked |= effects.iter().any(is_work);
    }

    pub(super) fn due(&mut self, now: Instant) -> Option<Vec<u8>> {
        match self.pending {
            Pending::Due(at) if now >= at => self.resend(),
            _ => None,
        }
    }

    pub(super) fn flush(&mut self) -> Option<Vec<u8>> {
        match self.pending {
            Pending::Due(_) => self.resend(),
            Pending::AwaitingResult { resent: false } => {
                self.pending = Pending::AwaitingResult { resent: true };
                self.worked = false;
                self.input.clone()
            }
            _ => None,
        }
    }

    fn resend(&mut self) -> Option<Vec<u8>> {
        self.pending = Pending::Idle;
        self.worked = false;
        self.input.clone()
    }
}

fn transient_failure(frame: &Value) -> bool {
    frame_type(frame) == Some("assistant")
        && frame.get("error").and_then(Value::as_str) == Some("server_error")
}

fn frame_type(frame: &Value) -> Option<&str> {
    frame.get("type").and_then(Value::as_str)
}

fn is_work(effect: &ClaudeRunnerEffect) -> bool {
    match effect {
        ClaudeRunnerEffect::PermissionRequest(_) | ClaudeRunnerEffect::SteerConsumed { .. } => true,
        ClaudeRunnerEffect::Fact(PublicWireFact::Compaction(_)) => true,
        ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::ToolActivity { .. },
        )) => true,
        ClaudeRunnerEffect::Fact(PublicWireFact::Event(event)) => matches!(
            event,
            NormalizedProviderEvent::Output { .. }
                | NormalizedProviderEvent::Thinking { .. }
                | NormalizedProviderEvent::ToolInputDelta { .. }
                | NormalizedProviderEvent::ToolOutputDelta { .. }
                | NormalizedProviderEvent::PlanProposed { .. }
                | NormalizedProviderEvent::ChildStarted { .. }
        ),
        _ => false,
    }
}

fn backoff(step: Duration, attempt: u8) -> Duration {
    let base = step * u32::from(attempt);
    let span = u64::try_from(step.as_millis() / 2).unwrap_or(u64::MAX);
    if span == 0 {
        return base;
    }
    base + Duration::from_millis(RandomState::new().build_hasher().finish() % span)
}
