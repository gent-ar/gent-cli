use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gent_ports::{AgentChatReadLedger, LedgerError};
use gent_runtime::AgentChatReadService;
use gent_types::{
    AgentChatConversationDetail, AgentChatConversationId, AgentChatConversationSummary,
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRun, AgentChatRunId,
    AgentChatRunState, AgentChatSelection, NormalizedTranscriptPage, ReceiptId,
};

use crate::agent_chat_api::{PromptCommitWake, PromptWake};
use crate::ordinary_lifecycle_cadence::{OrdinaryLifecycleCadence, OrdinaryPromptIngress, pair};
use crate::ordinary_lifecycle_control::{OrdinaryLifecycleControl, OrdinaryLifecyclePhase};
use crate::ordinary_lifecycle_router::{OrdinaryLifecycleHost, OrdinaryPublicLifecycleRouter};

#[derive(Clone)]
struct Ledger(AgentChatProvider);

impl AgentChatReadLedger for Ledger {
    fn read_agent_chat_summary(
        &self,
        _: &str,
    ) -> Result<AgentChatConversationSummary, LedgerError> {
        Ok(summary(self.0))
    }

    fn read_agent_chat_detail(&self, _: &str) -> Result<AgentChatConversationDetail, LedgerError> {
        Ok(AgentChatConversationDetail {
            summary: summary(self.0),
            current_run_id: "run".into(),
            runs: vec![AgentChatRun {
                run_id: "run-1".into(),
                parent_run_id: None,
                selection: selection(self.0),
                state: AgentChatRunState::Idle,
            }],
        })
    }

    fn read_agent_chat_transcript(
        &self,
        _: &str,
        _: Option<u64>,
        _: u16,
    ) -> Result<NormalizedTranscriptPage, LedgerError> {
        unreachable!()
    }
}

#[derive(Default)]
struct HostState {
    armed: bool,
    active_passes: usize,
}

#[derive(Clone)]
struct Host {
    provider: AgentChatProvider,
    events: Arc<Mutex<Vec<&'static str>>>,
    state: Arc<Mutex<HostState>>,
    delay: Duration,
    in_flight: Arc<AtomicUsize>,
    max_in_flight: Arc<AtomicUsize>,
}

impl OrdinaryLifecycleHost for Host {
    fn provider(&self) -> AgentChatProvider {
        self.provider
    }

    fn arm_authority_recovery(&mut self) -> Result<(), ()> {
        self.state.lock().unwrap().armed = true;
        self.events.lock().unwrap().push("recovery");
        Ok(())
    }

    fn wake(&mut self) -> Result<(), ()> {
        self.state.lock().unwrap().armed = true;
        self.events.lock().unwrap().push("wake");
        Ok(())
    }

    fn drive(&mut self) -> Result<(), ()> {
        let current = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_in_flight.fetch_max(current, Ordering::SeqCst);
        std::thread::sleep(self.delay);
        let mut state = self.state.lock().unwrap();
        state.armed = state.active_passes > 0;
        state.active_passes = state.active_passes.saturating_sub(1);
        self.events.lock().unwrap().push("drive");
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        Ok(())
    }

    fn needs_drive(&self) -> bool {
        self.state.lock().unwrap().armed
    }
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
async fn committed_prompt_notifies_and_drives_only_its_selected_provider() {
    let (control, cadence, mut wake, events, other_events, _, _) =
        cadence(AgentChatProvider::Codex, 0, Duration::ZERO);
    let task = tokio::spawn(cadence.run());
    wait_for(&events, 2).await;
    wait_for_ready(&control).await;
    events.lock().unwrap().clear();
    other_events.lock().unwrap().clear();

    wake.wake_after_prompt_commit(prompt()).unwrap();
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
async fn repeated_wakes_never_drive_a_host_concurrently() {
    let (control, cadence, mut wake, events, _, in_flight, max_in_flight) =
        cadence(AgentChatProvider::Codex, 0, Duration::from_millis(80));
    let task = tokio::spawn(cadence.run());
    wait_for(&events, 2).await;
    wait_for_ready(&control).await;
    events.lock().unwrap().clear();

    wake.wake_after_prompt_commit(prompt()).unwrap();
    wait_for_in_flight(&in_flight).await;
    let mut repeated_wake = wake.clone();
    let wake_task = tokio::task::spawn_blocking(move || {
        repeated_wake.wake_after_prompt_commit(prompt()).unwrap();
    });
    wait_for(&events, 4).await;
    wake_task.await.unwrap();
    assert_eq!(max_in_flight.load(Ordering::SeqCst), 1);
    task.abort();
}

type CadenceParts = (
    OrdinaryLifecycleControl,
    OrdinaryLifecycleCadence<Ledger>,
    OrdinaryPromptIngress<Ledger>,
    Arc<Mutex<Vec<&'static str>>>,
    Arc<Mutex<Vec<&'static str>>>,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
);

fn cadence(provider: AgentChatProvider, active_passes: usize, delay: Duration) -> CadenceParts {
    let events = Arc::new(Mutex::new(Vec::new()));
    let other_events = Arc::new(Mutex::new(Vec::new()));
    let selected = host(provider, active_passes, delay, Arc::clone(&events));
    let in_flight = Arc::clone(&selected.in_flight);
    let max_in_flight = Arc::clone(&selected.max_in_flight);
    let router = Arc::new(Mutex::new(
        OrdinaryPublicLifecycleRouter::new(
            AgentChatReadService::new(Ledger(provider)),
            vec![
                Box::new(host(
                    AgentChatProvider::Claude,
                    0,
                    delay,
                    Arc::clone(&other_events),
                )),
                Box::new(selected),
            ],
        )
        .unwrap(),
    ));
    let (control, wake, cadence) = pair(router);
    (
        control,
        cadence,
        wake,
        events,
        other_events,
        in_flight,
        max_in_flight,
    )
}

fn host(
    provider: AgentChatProvider,
    active_passes: usize,
    delay: Duration,
    events: Arc<Mutex<Vec<&'static str>>>,
) -> Host {
    Host {
        provider,
        events,
        state: Arc::new(Mutex::new(HostState {
            armed: false,
            active_passes,
        })),
        delay,
        in_flight: Arc::new(AtomicUsize::new(0)),
        max_in_flight: Arc::new(AtomicUsize::new(0)),
    }
}

async fn wait_for(events: &Arc<Mutex<Vec<&'static str>>>, length: usize) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while events.lock().unwrap().len() < length {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

async fn wait_for_in_flight(max_in_flight: &Arc<AtomicUsize>) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while max_in_flight.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

async fn wait_for_ready(control: &OrdinaryLifecycleControl) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while control.phase() != OrdinaryLifecyclePhase::Ready {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

fn prompt() -> PromptWake {
    PromptWake {
        conversation_id: AgentChatConversationId("conversation-1".into()),
        run_id: AgentChatRunId("run-1".into()),
        receipt_id: ReceiptId("receipt-1".into()),
        disposition: gent_types::AgentChatPromptDisposition::Send,
    }
}

fn summary(provider: AgentChatProvider) -> AgentChatConversationSummary {
    AgentChatConversationSummary {
        conversation_id: "conversation-1".into(),
        title: None,
        updated_at_unix_ms: 0,
        selection: selection(provider),
    }
}

fn selection(provider: AgentChatProvider) -> AgentChatSelection {
    AgentChatSelection {
        provider,
        model: "model".into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Ask,
    }
}
