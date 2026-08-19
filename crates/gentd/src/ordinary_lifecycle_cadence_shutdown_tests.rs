use std::sync::{Arc, Mutex};
use std::time::Duration;

use gent_ports::{AgentChatReadLedger, LedgerError};
use gent_runtime::AgentChatReadService;
use gent_types::{
    AgentChatConversationDetail, AgentChatConversationId, AgentChatConversationSummary,
    AgentChatEffort, AgentChatMode, AgentChatPromptDisposition, AgentChatProvider, AgentChatRun,
    AgentChatRunId, AgentChatRunState, AgentChatSelection, NormalizedTranscriptPage, ReceiptId,
};

use super::pair;
use crate::agent_chat_api::{PromptCommitWake, PromptWake};
use crate::ordinary_lifecycle_control::{OrdinaryLifecyclePhase, OrdinaryPromptAdmissionError};
use crate::ordinary_lifecycle_router::{OrdinaryLifecycleHost, OrdinaryPublicLifecycleRouter};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    Fresh,
    Recovering,
    Recovered,
    Draining,
    Stopped,
}

struct Host {
    state: State,
    events: Arc<Mutex<Vec<&'static str>>>,
}

#[derive(Clone)]
struct Ledger;

impl AgentChatReadLedger for Ledger {
    fn read_agent_chat_summary(
        &self,
        _: &str,
    ) -> Result<AgentChatConversationSummary, LedgerError> {
        Ok(AgentChatConversationSummary {
            conversation_id: "conversation-1".into(),
            title: None,
            updated_at_unix_ms: 0,
            selection: selection(),
        })
    }

    fn read_agent_chat_detail(&self, _: &str) -> Result<AgentChatConversationDetail, LedgerError> {
        Ok(AgentChatConversationDetail {
            summary: self.read_agent_chat_summary("")?,
            current_run_id: "run".into(),
            runs: vec![AgentChatRun {
                run_id: "run-1".into(),
                parent_run_id: None,
                selection: selection(),
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

impl OrdinaryLifecycleHost for Host {
    fn provider(&self) -> AgentChatProvider {
        AgentChatProvider::Codex
    }

    fn arm_authority_recovery(&mut self) -> Result<(), ()> {
        self.state = State::Recovering;
        self.events.lock().unwrap().push("recovery");
        Ok(())
    }

    fn wake(&mut self) -> Result<(), ()> {
        self.events.lock().unwrap().push("wake");
        Ok(())
    }

    fn drive(&mut self) -> Result<(), ()> {
        self.state = match self.state {
            State::Recovering => State::Recovered,
            State::Draining => State::Stopped,
            state => state,
        };
        self.events.lock().unwrap().push("drive");
        Ok(())
    }

    fn needs_drive(&self) -> bool {
        matches!(self.state, State::Recovering | State::Draining)
    }

    fn begin_shutdown_after_recovery(&mut self) -> Result<(), ()> {
        if self.state != State::Recovered {
            return Err(());
        }
        self.state = State::Draining;
        self.events.lock().unwrap().push("shutdown");
        Ok(())
    }

    fn shutdown_complete(&self) -> bool {
        self.state == State::Stopped
    }
}

#[tokio::test]
async fn shutdown_before_run_exits_unopened_without_a_recovery_wake() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let (control, _, cadence) = cadence(Arc::clone(&events));
    control.request_shutdown();

    cadence.run().await.unwrap();

    assert_eq!(control.phase(), OrdinaryLifecyclePhase::Draining);
    assert_eq!(
        control.acquire_prompt().map(|_| ()),
        Err(OrdinaryPromptAdmissionError::ShuttingDown)
    );
    assert!(events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ready_cadence_closes_admission_before_owner_proven_drain() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let (control, _, cadence) = cadence(Arc::clone(&events));
    let task = tokio::spawn(cadence.run());
    wait_for(&events, 2).await;

    assert_eq!(control.phase(), OrdinaryLifecyclePhase::Ready);
    control.request_shutdown();
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        &*events.lock().unwrap(),
        &["recovery", "drive", "shutdown", "drive"]
    );
}

#[tokio::test]
async fn permit_owns_the_post_commit_wake_before_shutdown_can_drain() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let (control, mut ingress, cadence) = cadence(Arc::clone(&events));
    let task = tokio::spawn(cadence.run());
    wait_for(&events, 2).await;
    let permit = ingress.acquire_prompt().unwrap();
    control.request_shutdown();
    ingress.wake_after_prompt_commit(prompt()).unwrap();

    let mut task = task;
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut task)
            .await
            .is_err()
    );
    drop(permit);
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        &*events.lock().unwrap(),
        &["recovery", "drive", "wake", "shutdown", "drive"]
    );
}

fn cadence(
    events: Arc<Mutex<Vec<&'static str>>>,
) -> (
    crate::ordinary_lifecycle_control::OrdinaryLifecycleControl,
    super::OrdinaryPromptIngress<Ledger>,
    super::OrdinaryLifecycleCadence<Ledger>,
) {
    let router = OrdinaryPublicLifecycleRouter::new(
        AgentChatReadService::new(Ledger),
        vec![Box::new(Host {
            state: State::Fresh,
            events,
        })],
    )
    .unwrap();
    pair(Arc::new(Mutex::new(router)))
}

async fn wait_for(events: &Arc<Mutex<Vec<&'static str>>>, count: usize) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while events.lock().unwrap().len() < count {
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
        disposition: AgentChatPromptDisposition::Send,
    }
}

fn selection() -> AgentChatSelection {
    AgentChatSelection {
        provider: AgentChatProvider::Codex,
        model: "model".into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Ask,
    }
}
