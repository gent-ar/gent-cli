use std::time::Duration;

use super::{OrdinaryLifecycleControl, OrdinaryLifecyclePhase, OrdinaryPromptAdmissionError};

#[test]
fn admission_starts_closed_and_opens_only_after_recovery() {
    let control = OrdinaryLifecycleControl::new();

    assert_eq!(control.phase(), OrdinaryLifecyclePhase::Recovering);
    assert_eq!(
        control.acquire_prompt().map(|_| ()),
        Err(OrdinaryPromptAdmissionError::RecoveryInProgress)
    );
    control.open_after_recovery();
    assert_eq!(control.phase(), OrdinaryLifecyclePhase::Ready);
    drop(control.acquire_prompt().unwrap());
}

#[test]
fn shutdown_wins_the_recovery_race_and_never_reopens_admission() {
    let control = OrdinaryLifecycleControl::new();

    control.request_shutdown();
    control.open_after_recovery();
    assert_eq!(control.phase(), OrdinaryLifecyclePhase::Draining);
    assert_eq!(
        control.acquire_prompt().map(|_| ()),
        Err(OrdinaryPromptAdmissionError::ShuttingDown)
    );
}

#[tokio::test]
async fn shutdown_waits_for_a_permit_that_began_before_closure() {
    let control = OrdinaryLifecycleControl::new();
    control.open_after_recovery();
    let permit = control.acquire_prompt().unwrap();
    control.request_shutdown();
    assert_eq!(
        control.acquire_prompt().map(|_| ()),
        Err(OrdinaryPromptAdmissionError::ShuttingDown)
    );

    let waiting = control.clone();
    let mut task = tokio::spawn(async move { waiting.wait_for_permits().await });
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut task)
            .await
            .is_err()
    );
    drop(permit);
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn shutdown_wait_observes_a_prior_or_later_request() {
    let control = OrdinaryLifecycleControl::new();
    let waiting = control.clone();
    let task = tokio::spawn(async move { waiting.shutdown_requested().await });

    tokio::time::sleep(Duration::from_millis(5)).await;
    control.request_shutdown();
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
    control.shutdown_requested().await;
}
