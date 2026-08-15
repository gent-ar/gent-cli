use super::*;

fn run<'a>(
    automation_id: &'a str,
    run_id: &'a str,
    state: AutomationRunState,
) -> AutomationRun<'a> {
    AutomationRun {
        automation_id,
        run_id,
        state,
    }
}

#[test]
fn observer_mode_hard_disables_all_trigger_paths() {
    let policy = WebhookAuthPolicy::Bearer(BearerTokenDigest::from_token("secret"));
    assert_eq!(
        scheduler_state(AutomationMode::ObserverDisabled),
        SchedulerState::HardDisabled
    );
    assert_eq!(
        admit_run(
            AutomationMode::ObserverDisabled,
            ConcurrencyLimit(1),
            &[],
            &run("nightly", "one", AutomationRunState::Queued),
        ),
        RunAdmission::RejectedObserver
    );
    assert_eq!(
        authorize_webhook(
            AutomationMode::ObserverDisabled,
            &policy,
            WebhookCredentials::Bearer("secret"),
        ),
        WebhookAdmission::RejectedObserver
    );
}

#[test]
fn concurrency_is_per_automation_and_duplicate_safe() {
    let existing = [
        run("nightly", "active", AutomationRunState::Running),
        run("other", "other-active", AutomationRunState::Running),
    ];
    assert_eq!(
        admit_run(
            AutomationMode::Authoritative,
            ConcurrencyLimit(1),
            &existing,
            &run("nightly", "next", AutomationRunState::Queued),
        ),
        RunAdmission::Blocked {
            active: 1,
            limit: ConcurrencyLimit(1)
        }
    );
    assert_eq!(
        admit_run(
            AutomationMode::Authoritative,
            ConcurrencyLimit(1),
            &existing,
            &run("other-two", "new", AutomationRunState::Queued),
        ),
        RunAdmission::Start
    );
    assert_eq!(
        admit_run(
            AutomationMode::Authoritative,
            ConcurrencyLimit(2),
            &existing,
            &run("nightly", "active", AutomationRunState::Queued),
        ),
        RunAdmission::Duplicate { run_id: "active" }
    );
}

#[test]
fn missed_schedules_have_bounded_catch_up_and_running_rows_reconcile() {
    assert_eq!(
        decide_missed_schedule(MissedSchedulePolicy::CatchUpOne, &[10, 20, 30]),
        MissedScheduleDecision::CatchUp {
            scheduled_for: 30,
            skipped_additional: 2
        }
    );
    assert_eq!(
        decide_missed_schedule(MissedSchedulePolicy::Skip, &[10, 20]),
        MissedScheduleDecision::Skipped { occurrences: 2 }
    );
    assert_eq!(
        reconcile_after_crash(AutomationRunState::Running),
        RecoveryDecision::MarkInterrupted
    );
    assert_eq!(
        reconcile_after_crash(AutomationRunState::Succeeded),
        RecoveryDecision::LeaveUntouched
    );
}

#[test]
fn webhook_requires_valid_secret_without_retaining_it() {
    let policy = WebhookAuthPolicy::Bearer(BearerTokenDigest::from_token("long-random-token"));
    assert_eq!(
        authorize_webhook(
            AutomationMode::Authoritative,
            &policy,
            WebhookCredentials::Missing,
        ),
        WebhookAdmission::MissingCredentials
    );
    assert_eq!(
        authorize_webhook(
            AutomationMode::Authoritative,
            &policy,
            WebhookCredentials::Bearer("wrong"),
        ),
        WebhookAdmission::RejectedCredentials
    );
    assert_eq!(
        authorize_webhook(
            AutomationMode::Authoritative,
            &policy,
            WebhookCredentials::Bearer("long-random-token"),
        ),
        WebhookAdmission::Authorized
    );
    assert_eq!(format!("{policy:?}"), "Bearer(BearerTokenDigest(REDACTED))");
}
