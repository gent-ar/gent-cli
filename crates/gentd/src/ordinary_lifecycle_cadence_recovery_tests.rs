use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use gent_types::{AgentChatProvider, AgentChatRunId};

use super::{AsyncHost, cadence, prompt, wait_for, wait_for_ready};
use crate::agent_chat_api::PromptCommitWake;

#[derive(Debug)]
struct TurnBoundHost {
    current: Arc<Mutex<String>>,
    attempts: Arc<Mutex<Vec<String>>>,
    signals: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl crate::ordinary_lifecycle_cadence::AsyncOrdinaryLifecycleHost for TurnBoundHost {
    async fn activate_recovery(&mut self) -> Result<(), String> {
        Ok(())
    }
    async fn drive_once(&mut self) -> Result<bool, String> {
        Ok(false)
    }
    async fn begin_shutdown_after_recovery(&mut self) -> Result<(), String> {
        Ok(())
    }
    async fn interrupt_claurst_run(&mut self, _: &str) -> Result<(), String> {
        panic!("a bound request must never fall back to a run-wide signal")
    }
    async fn interrupt_claurst_turn(&mut self, _: &str, turn: &str) -> Result<(), String> {
        self.attempts.lock().unwrap().push(turn.into());
        if self.current.lock().unwrap().as_str() != turn {
            return Err("stale turn".into());
        }
        self.signals.lock().unwrap().push(turn.into());
        Ok(())
    }
    fn shutdown_complete(&self) -> bool {
        true
    }
}

#[tokio::test]
async fn queued_interrupt_turn_checks_binding_when_the_async_owner_consumes_it() {
    let (_, cadence, wake, _, _, _, _) = cadence(AgentChatProvider::Codex, 0, Duration::ZERO);
    let current = Arc::new(Mutex::new("old-turn".to_owned()));
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let signals = Arc::new(Mutex::new(Vec::new()));
    wake.attach_async_claurst(Box::new(TurnBoundHost {
        current: Arc::clone(&current),
        attempts: Arc::clone(&attempts),
        signals: Arc::clone(&signals),
    }))
    .await
    .unwrap();
    {
        let mut queued = wake.async_claurst_interrupts.lock().unwrap();
        for turn in ["old-turn", "new-turn"] {
            queued.push_back(crate::ordinary_lifecycle_cadence::PendingClaurstInterrupt {
                run_id: "same-run".into(),
                expected_turn_id: Some(turn.into()),
                goal_stop: None,
            });
        }
    }
    *current.lock().unwrap() = "new-turn".into();
    let task = tokio::spawn(cadence.run());
    tokio::time::timeout(Duration::from_secs(1), async {
        while attempts.lock().unwrap().len() != 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(&*signals.lock().unwrap(), &["new-turn"]);
    task.abort();
}

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
    wait_for(&events, 2).await;
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
#[tokio::test]
async fn steering_wakes_a_native_steer_owner_but_interrupts_claurst() {
    let (_, cadence, wake, events, _, _, _) = cadence(AgentChatProvider::Codex, 0, Duration::ZERO);
    let interrupts = Arc::new(Mutex::new(Vec::new()));
    wake.attach_async_claurst(Box::new(AsyncHost {
        events: Arc::new(Mutex::new(Vec::new())),
        interrupts: Arc::clone(&interrupts),
        active: false,
        stopping: false,
    }))
    .await
    .unwrap();
    let task = tokio::spawn(cadence.run());
    wait_for(&events, 1).await;

    wake.steer_run(AgentChatProvider::Codex, prompt()).unwrap();
    wait_for(&events, 2).await;
    assert!(events.lock().unwrap().contains(&"wake"));
    assert!(interrupts.lock().unwrap().is_empty());

    let mut claurst = prompt();
    claurst.run_id = AgentChatRunId("run-claurst".into());
    wake.steer_run(AgentChatProvider::Claurst, claurst).unwrap();
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

#[tokio::test]
async fn a_claurst_warm_up_reaches_its_async_owner_once_it_is_idle() {
    let (control, cadence, wake, _, _, _, _) = cadence(AgentChatProvider::Codex, 0, Duration::ZERO);
    let task = tokio::spawn(cadence.run());
    wait_for_ready(&control).await;
    let request = crate::provider_warm_pool::WarmPoolRequest {
        now: std::time::Instant::now(),
        target: Some(crate::provider_warm_pool::WarmTarget {
            conversation_id: "conversation-claurst".into(),
            run_id: "run-claurst".into(),
        }),
    };
    assert_eq!(
        wake.tend_warm_pool(Some(AgentChatProvider::Claurst), &request)
            .unwrap(),
        Some(gent_protocol::ProviderWarmOutcome::Unavailable)
    );
    let warms = Arc::new(Mutex::new(Vec::new()));
    wake.attach_async_claurst(Box::new(AsyncHost {
        events: Arc::new(Mutex::new(Vec::new())),
        interrupts: Arc::clone(&warms),
        active: false,
        stopping: false,
    }))
    .await
    .unwrap();
    assert_eq!(
        wake.tend_warm_pool(Some(AgentChatProvider::Claurst), &request)
            .unwrap(),
        Some(gent_protocol::ProviderWarmOutcome::Warming)
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        while warms.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        &*warms.lock().unwrap(),
        &["warm:conversation-claurst:run-claurst"]
    );
    task.abort();
}

#[derive(Debug)]
struct StalledWarmHost;

#[async_trait::async_trait]
impl crate::ordinary_lifecycle_cadence::AsyncOrdinaryLifecycleHost for StalledWarmHost {
    async fn activate_recovery(&mut self) -> Result<(), String> {
        Ok(())
    }
    async fn drive_once(&mut self) -> Result<bool, String> {
        Ok(false)
    }
    async fn begin_shutdown_after_recovery(&mut self) -> Result<(), String> {
        Ok(())
    }
    async fn warm_claurst(&mut self, _: &str, _: &str) -> Result<(), String> {
        std::future::pending().await
    }
    fn shutdown_complete(&self) -> bool {
        true
    }
}

#[tokio::test]
async fn a_slow_claurst_warm_up_never_blocks_other_provider_prompts() {
    let (control, cadence, mut wake, events, _, _, _) =
        cadence(AgentChatProvider::Codex, 0, Duration::ZERO);
    let task = tokio::spawn(cadence.run());
    wait_for_ready(&control).await;
    wake.attach_async_claurst(Box::new(StalledWarmHost))
        .await
        .unwrap();
    let request = crate::provider_warm_pool::WarmPoolRequest {
        now: std::time::Instant::now(),
        target: Some(crate::provider_warm_pool::WarmTarget {
            conversation_id: "conversation-claurst".into(),
            run_id: "run-claurst".into(),
        }),
    };
    assert_eq!(
        wake.tend_warm_pool(Some(AgentChatProvider::Claurst), &request)
            .unwrap(),
        Some(gent_protocol::ProviderWarmOutcome::Warming)
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    events.lock().unwrap().clear();
    wake.wake_after_prompt_commit(prompt()).unwrap();
    wait_for(&events, 2).await;
    assert!(events.lock().unwrap().contains(&"drive"));
    task.abort();
}
