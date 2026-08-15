//! Stateless decisions for a future automation executor.
//!
//! The executor, clock, persistence, and webhook transport deliberately live outside
//! this crate. In observer mode every trigger is rejected before it can have effects.

use gent_types::AutomationExecutionPhase;
use sha2::{Digest, Sha256};

/// Authority state supplied by the daemon composition root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutomationMode {
    /// Hard disable: no schedules, recovery jobs, or webhooks may fire.
    ObserverDisabled,
    /// Policies may admit work once a future executor has authority.
    Authoritative,
}

/// The only scheduler state available in observer mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulerState {
    HardDisabled,
    Eligible,
}

/// Maps authority to the scheduler's allowed state without starting anything.
#[must_use]
pub const fn scheduler_state(mode: AutomationMode) -> SchedulerState {
    match mode {
        AutomationMode::ObserverDisabled => SchedulerState::HardDisabled,
        AutomationMode::Authoritative => SchedulerState::Eligible,
    }
}

/// One durable run's state as viewed by the pure policy.
/// Compatibility name for the durable execution phase shared with the ledger.
pub type AutomationRunState = AutomationExecutionPhase;

/// Minimal durable projection required for concurrency and recovery decisions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutomationRun<'a> {
    pub automation_id: &'a str,
    pub run_id: &'a str,
    pub state: AutomationRunState,
}

/// A per-automation concurrency limit. Zero means schedules are paused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConcurrencyLimit(pub u16);

/// Result of a proposed run, with no persistence or process side effect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunAdmission<'a> {
    RejectedObserver,
    Paused,
    Duplicate {
        run_id: &'a str,
    },
    Blocked {
        active: u16,
        limit: ConcurrencyLimit,
    },
    Start,
}

/// Applies a per-automation gate. Other automations never consume this limit.
#[must_use]
pub fn admit_run<'a>(
    mode: AutomationMode,
    limit: ConcurrencyLimit,
    existing: &[AutomationRun<'a>],
    candidate: &AutomationRun<'a>,
) -> RunAdmission<'a> {
    if mode == AutomationMode::ObserverDisabled {
        return RunAdmission::RejectedObserver;
    }
    if limit.0 == 0 {
        return RunAdmission::Paused;
    }
    if let Some(run) = existing.iter().find(|run| run.run_id == candidate.run_id) {
        return RunAdmission::Duplicate { run_id: run.run_id };
    }
    let active = existing
        .iter()
        .filter(|run| run.automation_id == candidate.automation_id && occupies_slot(run.state))
        .count();
    let active = u16::try_from(active).unwrap_or(u16::MAX);
    if active >= limit.0 {
        RunAdmission::Blocked { active, limit }
    } else {
        RunAdmission::Start
    }
}

const fn occupies_slot(state: AutomationRunState) -> bool {
    matches!(
        state,
        AutomationRunState::Queued | AutomationRunState::Running
    )
}

/// Declared response to cron occurrences that were missed while the host was down.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MissedSchedulePolicy {
    Skip,
    CatchUpOne,
}

/// A scheduling decision; the executor records the result as a receipt/event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MissedScheduleDecision {
    None,
    Skipped {
        occurrences: u32,
    },
    CatchUp {
        scheduled_for: i64,
        skipped_additional: u32,
    },
}

/// Uses caller-supplied occurrence times, so it needs neither a clock nor cron parser.
#[must_use]
pub fn decide_missed_schedule(
    policy: MissedSchedulePolicy,
    missed_occurrences: &[i64],
) -> MissedScheduleDecision {
    let count = u32::try_from(missed_occurrences.len()).unwrap_or(u32::MAX);
    match (policy, missed_occurrences.last().copied()) {
        (_, None) => MissedScheduleDecision::None,
        (MissedSchedulePolicy::Skip, Some(_)) => {
            MissedScheduleDecision::Skipped { occurrences: count }
        }
        (MissedSchedulePolicy::CatchUpOne, Some(scheduled_for)) => {
            MissedScheduleDecision::CatchUp {
                scheduled_for,
                skipped_additional: count.saturating_sub(1),
            }
        }
    }
}

/// Crash recovery never resumes a zombie execution without an executor decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryDecision {
    LeaveUntouched,
    MarkInterrupted,
}

#[must_use]
pub const fn reconcile_after_crash(state: AutomationRunState) -> RecoveryDecision {
    match state {
        AutomationRunState::Running => RecoveryDecision::MarkInterrupted,
        AutomationRunState::Queued
        | AutomationRunState::Succeeded
        | AutomationRunState::Failed
        | AutomationRunState::Interrupted => RecoveryDecision::LeaveUntouched,
    }
}

/// Stored opaque verification material; the raw bearer token is never retained here.
#[derive(Clone, Eq, PartialEq)]
pub struct BearerTokenDigest([u8; 32]);

impl std::fmt::Debug for BearerTokenDigest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BearerTokenDigest(REDACTED)")
    }
}

impl BearerTokenDigest {
    #[must_use]
    pub fn from_token(token: &str) -> Self {
        Self(Sha256::digest(token.as_bytes()).into())
    }

    fn matches(&self, token: &str) -> bool {
        let candidate: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        self.0
            .iter()
            .zip(candidate)
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
    }
}

/// Webhook ingress must name authentication; an unauthenticated policy is impossible.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WebhookAuthPolicy {
    Bearer(BearerTokenDigest),
}

/// Transport-normalized authorization material. It is intentionally short-lived.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WebhookCredentials<'a> {
    Missing,
    Bearer(&'a str),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WebhookAdmission {
    RejectedObserver,
    MissingCredentials,
    RejectedCredentials,
    Authorized,
}

/// Authorizes a webhook without opening a listener or invoking an automation.
#[must_use]
pub fn authorize_webhook(
    mode: AutomationMode,
    policy: &WebhookAuthPolicy,
    credentials: WebhookCredentials<'_>,
) -> WebhookAdmission {
    if mode == AutomationMode::ObserverDisabled {
        return WebhookAdmission::RejectedObserver;
    }
    match (policy, credentials) {
        (_, WebhookCredentials::Missing) => WebhookAdmission::MissingCredentials,
        (WebhookAuthPolicy::Bearer(expected), WebhookCredentials::Bearer(value)) => {
            if expected.matches(value) {
                WebhookAdmission::Authorized
            } else {
                WebhookAdmission::RejectedCredentials
            }
        }
    }
}
