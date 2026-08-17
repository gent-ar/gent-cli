//! Pure scheduling policy for an explicitly opted-in runtime updater.

/// Durable-free configuration for bounded periodic update checks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeUpdateSchedule {
    pub enabled: bool,
    pub interval_seconds: u64,
    pub maximum_backoff_seconds: u64,
}

/// Content-free state a composition layer can persist beside its update cache.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeUpdateScheduleState {
    pub next_check_at_unix_seconds: u64,
    pub consecutive_failures: u8,
}

/// Decision made without a clock, process, network, or ledger dependency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeUpdateScheduleDecision {
    Disabled,
    Busy,
    WaitUntil(u64),
    CheckNow,
}

/// Result supplied by a concrete update source after one bounded check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeUpdateCheckOutcome {
    Verified,
    Unavailable,
}

/// Determines whether one opted-in, authority-approved updater may check now.
#[must_use]
pub fn schedule_runtime_update_check(
    schedule: RuntimeUpdateSchedule,
    state: RuntimeUpdateScheduleState,
    now_unix_seconds: u64,
    authority_approved: bool,
    idle: bool,
) -> RuntimeUpdateScheduleDecision {
    if !authority_approved || !schedule.enabled {
        RuntimeUpdateScheduleDecision::Disabled
    } else if !idle {
        RuntimeUpdateScheduleDecision::Busy
    } else if now_unix_seconds < state.next_check_at_unix_seconds {
        RuntimeUpdateScheduleDecision::WaitUntil(state.next_check_at_unix_seconds)
    } else {
        RuntimeUpdateScheduleDecision::CheckNow
    }
}

/// Advances a schedule after a bounded check without retrying synchronously.
#[must_use]
pub fn record_runtime_update_check(
    schedule: RuntimeUpdateSchedule,
    state: RuntimeUpdateScheduleState,
    now_unix_seconds: u64,
    outcome: RuntimeUpdateCheckOutcome,
) -> RuntimeUpdateScheduleState {
    let failures = match outcome {
        RuntimeUpdateCheckOutcome::Verified => 0,
        RuntimeUpdateCheckOutcome::Unavailable => state.consecutive_failures.saturating_add(1),
    };
    let delay = match outcome {
        RuntimeUpdateCheckOutcome::Verified => schedule.interval_seconds,
        RuntimeUpdateCheckOutcome::Unavailable => backoff(schedule, failures),
    };
    RuntimeUpdateScheduleState {
        next_check_at_unix_seconds: now_unix_seconds.saturating_add(delay),
        consecutive_failures: failures,
    }
}

fn backoff(schedule: RuntimeUpdateSchedule, failures: u8) -> u64 {
    let multiplier = 1_u64
        .checked_shl(u32::from(failures.min(63)))
        .unwrap_or(u64::MAX);
    schedule.interval_seconds.saturating_mul(multiplier).min(
        schedule
            .maximum_backoff_seconds
            .max(schedule.interval_seconds),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEDULE: RuntimeUpdateSchedule = RuntimeUpdateSchedule {
        enabled: true,
        interval_seconds: 60,
        maximum_backoff_seconds: 240,
    };

    #[test]
    fn observer_and_busy_hosts_never_check() {
        let state = RuntimeUpdateScheduleState {
            next_check_at_unix_seconds: 0,
            consecutive_failures: 0,
        };
        assert_eq!(
            schedule_runtime_update_check(SCHEDULE, state, 1, false, true),
            RuntimeUpdateScheduleDecision::Disabled
        );
        assert_eq!(
            schedule_runtime_update_check(SCHEDULE, state, 1, true, false),
            RuntimeUpdateScheduleDecision::Busy
        );
    }

    #[test]
    fn checks_only_when_due_and_idle() {
        let state = RuntimeUpdateScheduleState {
            next_check_at_unix_seconds: 10,
            consecutive_failures: 0,
        };
        assert_eq!(
            schedule_runtime_update_check(SCHEDULE, state, 9, true, true),
            RuntimeUpdateScheduleDecision::WaitUntil(10)
        );
        assert_eq!(
            schedule_runtime_update_check(SCHEDULE, state, 10, true, true),
            RuntimeUpdateScheduleDecision::CheckNow
        );
    }

    #[test]
    fn failures_back_off_and_verified_checks_reset_the_failure_count() {
        let first = record_runtime_update_check(
            SCHEDULE,
            RuntimeUpdateScheduleState {
                next_check_at_unix_seconds: 0,
                consecutive_failures: 0,
            },
            10,
            RuntimeUpdateCheckOutcome::Unavailable,
        );
        assert_eq!(first.next_check_at_unix_seconds, 130);
        let capped = record_runtime_update_check(
            SCHEDULE,
            RuntimeUpdateScheduleState {
                consecutive_failures: 9,
                ..first
            },
            130,
            RuntimeUpdateCheckOutcome::Unavailable,
        );
        assert_eq!(capped.next_check_at_unix_seconds, 370);
        let verified =
            record_runtime_update_check(SCHEDULE, capped, 370, RuntimeUpdateCheckOutcome::Verified);
        assert_eq!(verified.consecutive_failures, 0);
        assert_eq!(verified.next_check_at_unix_seconds, 430);
    }
}
