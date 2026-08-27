use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use gent_types::AgentChatProvider;

use super::{AsyncHost, cadence, wait_for, wait_for_ready};
use crate::agent_chat_api::PromptCommitWake;

#[tokio::test]
async fn recovery_drives_once_then_waits_without_idle_polling() {
    let (control, cadence, _, events, _, _, _) =
        cadence(AgentChatProvider::Codex, 0, Duration::ZERO);
    let task = tokio::spawn(cadence.run());
    wait_for(&events, 2).await;
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(&*events.lock().unwrap(), &["recovery", "drive"]);
    drop(control.acquire_prompt().unwrap());
    task.abort();
}

#[tokio::test]
async fn restart_admission_waits_for_recovery_before_the_first_prompt() {
    let (control, cadence, _, events, _, _, _) =
        cadence(AgentChatProvider::Codex, 0, Duration::ZERO);
    let waiting = control.clone();
    let mut readiness = tokio::spawn(async move { waiting.wait_until_ready().await });

    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut readiness)
            .await
            .is_err()
    );
    let task = tokio::spawn(cadence.run());
    readiness.await.unwrap().unwrap();
    assert_eq!(
        control.phase(),
        crate::ordinary_lifecycle_control::OrdinaryLifecyclePhase::Ready
    );
    drop(control.acquire_prompt().unwrap());
    assert_eq!(&*events.lock().unwrap(), &["recovery", "drive"]);
    task.abort();
}
#[tokio::test]
async fn committed_prompt_notifies_and_drives_only_its_selected_provider() {
    let (control, cadence, mut wake, events, other_events, _, _) =
        cadence(AgentChatProvider::Codex, 0, Duration::ZERO);
    let task = tokio::spawn(cadence.run());
    wait_for(&events, 2).await;
    wait_for_ready(&control).await;
    events.lock().unwrap().clear();
    other_events.lock().unwrap().clear();
    wake.wake_after_prompt_commit(super::prompt()).unwrap();
    wait_for(&events, 2).await;
    assert_eq!(&*events.lock().unwrap(), &["wake", "drive"]);
    assert!(other_events.lock().unwrap().is_empty());
    task.abort();
}
#[tokio::test]
async fn active_host_repeats_until_it_settles_then_stops() {
    let (_, cadence, _, events, _, _, _) = cadence(AgentChatProvider::Codex, 1, Duration::ZERO);
    let task = tokio::spawn(cadence.run());
    wait_for(&events, 3).await;
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(&*events.lock().unwrap(), &["recovery", "drive", "drive"]);
    task.abort();
}
#[tokio::test]
async fn async_claurst_owner_is_recovered_and_driven_by_the_same_ordinary_cadence() {
    let (_, cadence, wake, _, _, _, _) = cadence(AgentChatProvider::Codex, 0, Duration::ZERO);
    let events = Arc::new(Mutex::new(Vec::new()));
    wake.attach_async_claurst(Box::new(AsyncHost {
        events: Arc::clone(&events),
        interrupts: Arc::new(Mutex::new(Vec::new())),
        active: true,
        stopping: false,
    }))
    .await
    .unwrap();
    let task = tokio::spawn(cadence.run());
    wait_for(&events, 2).await;
    assert_eq!(&*events.lock().unwrap(), &["recovery", "drive"]);
    task.abort();
}

#[tokio::test]
async fn accepted_claurst_interrupt_is_delivered_to_its_async_owner_without_a_terminal_claim() {
    let (_, cadence, wake, _, _, _, _) = cadence(AgentChatProvider::Codex, 0, Duration::ZERO);
    let events = Arc::new(Mutex::new(Vec::new()));
    let interrupts = Arc::new(Mutex::new(Vec::new()));
    wake.attach_async_claurst(Box::new(AsyncHost {
        events,
        interrupts: Arc::clone(&interrupts),
        active: false,
        stopping: false,
    }))
    .await
    .unwrap();
    let task = tokio::spawn(cadence.run());
    wake.interrupt_run(AgentChatProvider::Claurst, "run-claurst")
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while interrupts.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(&*interrupts.lock().unwrap(), &["run-claurst"]);
    task.abort();
}
