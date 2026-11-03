use std::time::Duration;

use super::{OrdinaryLifecycleControl, OrdinaryLifecyclePhase, OrdinaryPromptAdmissionError};

#[test]
fn quiesce_rejects_active_operations_without_closing_admission() {
    let control = OrdinaryLifecycleControl::new();
    control.open_after_recovery();
    let permit = control.acquire_operation().unwrap();
    assert!(
        control
            .quiesce_if_idle(|| panic!("busy owner must not close durable ingress"))
            .is_err()
    );
    assert_eq!(control.phase(), OrdinaryLifecyclePhase::Ready);
    drop(permit);
    control.quiesce_if_idle(|| Ok(())).unwrap();
    assert!(control.is_quiesced());
    assert_eq!(
        control.acquire_operation().map(|_| ()),
        Err(OrdinaryPromptAdmissionError::ShuttingDown)
    );
    control
        .quiesce_if_idle(|| panic!("same owner retry must preserve closure"))
        .unwrap();
}

#[test]
fn failed_idle_proof_preserves_ready_admission() {
    let control = OrdinaryLifecycleControl::new();
    control.open_after_recovery();
    assert!(
        control
            .quiesce_if_idle(|| Err("active native work".into()))
            .is_err()
    );
    assert!(!control.is_quiesced());
    drop(control.acquire_operation().unwrap());
}

#[test]
fn quiesce_linearizes_before_a_racing_new_operation() {
    let control = OrdinaryLifecycleControl::new();
    control.open_after_recovery();
    let competing = control.clone();
    let (started, observed) = std::sync::mpsc::channel();
    let mut task = None;
    control
        .quiesce_if_idle(|| {
            task = Some(std::thread::spawn(move || {
                started.send(()).unwrap();
                competing.acquire_operation().map(|_| ())
            }));
            observed.recv().unwrap();
            Ok(())
        })
        .unwrap();
    assert_eq!(
        task.unwrap().join().unwrap(),
        Err(OrdinaryPromptAdmissionError::ShuttingDown)
    );
}

#[tokio::test]
async fn quiesce_does_not_authorize_exit_until_the_response_is_flushed() {
    let control = OrdinaryLifecycleControl::new();
    control.open_after_recovery();
    control.quiesce_if_idle(|| Ok(())).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(5), control.quiesce_acknowledged())
            .await
            .is_err()
    );
    control.acknowledge_quiesce();
    tokio::time::timeout(Duration::from_secs(1), control.quiesce_acknowledged())
        .await
        .unwrap();
}

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
fn startup_metadata_operations_are_counted_without_opening_prompt_admission() {
    let control = OrdinaryLifecycleControl::new();
    let operation = control.acquire_operation().unwrap();
    assert!(control.acquire_prompt().is_err());
    assert!(
        control
            .quiesce_if_idle(|| panic!("recovering owner must not close"))
            .is_err()
    );
    control.open_after_recovery();
    assert!(
        control
            .quiesce_if_idle(|| panic!("startup operation remains active"))
            .is_err()
    );
    drop(operation);
    control.quiesce_if_idle(|| Ok(())).unwrap();
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
