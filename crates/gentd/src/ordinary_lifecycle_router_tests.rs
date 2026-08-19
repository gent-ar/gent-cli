use std::sync::{Arc, Mutex};

use gent_ports::{AgentChatReadLedger, LedgerError};
use gent_runtime::AgentChatReadService;
use gent_types::{
    AgentChatConversationDetail, AgentChatConversationId, AgentChatConversationSummary,
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRun, AgentChatRunId,
    AgentChatRunState, AgentChatSelection, NormalizedTranscriptPage, ReceiptId,
};

use crate::agent_chat_api::{PromptCommitWake, PromptWake};
use crate::ordinary_lifecycle_router::{
    OrdinaryLifecycleHost, OrdinaryLifecycleRouterError, OrdinaryProviderHost,
    OrdinaryPublicLifecycleRouter,
};
use crate::private_lifecycle_loop::PrivateLifecycleOwner;
use crate::provider_lifecycle_host::ProviderLifecycleHost;

#[derive(Clone)]
struct Ledger {
    provider: AgentChatProvider,
}

impl AgentChatReadLedger for Ledger {
    fn read_agent_chat_summary(
        &self,
        _: &str,
    ) -> Result<AgentChatConversationSummary, LedgerError> {
        Ok(summary(self.provider))
    }

    fn read_agent_chat_detail(&self, _: &str) -> Result<AgentChatConversationDetail, LedgerError> {
        Ok(AgentChatConversationDetail {
            summary: summary(self.provider),
            runs: vec![AgentChatRun {
                run_id: "run-1".into(),
                parent_run_id: None,
                selection: selection(self.provider),
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

#[derive(Clone)]
struct Host {
    provider: AgentChatProvider,
    calls: Arc<Mutex<Vec<&'static str>>>,
}

impl OrdinaryLifecycleHost for Host {
    fn provider(&self) -> AgentChatProvider {
        self.provider
    }

    fn wake(&mut self) -> Result<(), ()> {
        self.calls.lock().unwrap().push("wake");
        Ok(())
    }

    fn drive(&mut self) -> Result<(), ()> {
        self.calls.lock().unwrap().push("drive");
        Ok(())
    }

    fn needs_drive(&self) -> bool {
        false
    }
}

#[test]
fn committed_prompt_routes_only_from_its_durable_run_selection() {
    let claude_calls = Arc::new(Mutex::new(Vec::new()));
    let codex_calls = Arc::new(Mutex::new(Vec::new()));
    let mut router = router(
        AgentChatProvider::Codex,
        host(AgentChatProvider::Claude, Arc::clone(&claude_calls)),
        host(AgentChatProvider::Codex, Arc::clone(&codex_calls)),
    );

    router.wake_after_prompt_commit(wake()).unwrap();
    assert!(claude_calls.lock().unwrap().is_empty());
    assert_eq!(&*codex_calls.lock().unwrap(), &["wake"]);
}

#[test]
fn unavailable_private_bridge_host_fails_closed_without_waking_another_provider() {
    let claude_calls = Arc::new(Mutex::new(Vec::new()));
    let mut router = OrdinaryPublicLifecycleRouter::new(
        AgentChatReadService::new(Ledger {
            provider: AgentChatProvider::Claurst,
        }),
        vec![host(AgentChatProvider::Claude, Arc::clone(&claude_calls))],
    )
    .unwrap();

    assert_eq!(
        router.wake_after_prompt_commit(wake()),
        Err(OrdinaryLifecycleRouterError::HostUnavailable(
            AgentChatProvider::Claurst
        ))
    );
    assert!(claude_calls.lock().unwrap().is_empty());
}

#[test]
fn cadence_drives_each_private_host_at_most_once() {
    let claude_calls = Arc::new(Mutex::new(Vec::new()));
    let codex_calls = Arc::new(Mutex::new(Vec::new()));
    let mut router = router(
        AgentChatProvider::Claude,
        host(AgentChatProvider::Claude, Arc::clone(&claude_calls)),
        host(AgentChatProvider::Codex, Arc::clone(&codex_calls)),
    );

    assert!(!router.drive_once().unwrap());
    assert_eq!(&*claude_calls.lock().unwrap(), &["drive"]);
    assert_eq!(&*codex_calls.lock().unwrap(), &["drive"]);
}

#[test]
fn concrete_lifecycle_adapter_only_drives_after_commit_wake() {
    let owner = Owner::default();
    let calls = Arc::clone(&owner.calls);
    let mut host =
        OrdinaryProviderHost::new(AgentChatProvider::Codex, ProviderLifecycleHost::new(owner));

    assert_eq!(host.provider(), AgentChatProvider::Codex);
    host.wake().unwrap();
    assert!(calls.lock().unwrap().is_empty());
    host.drive().unwrap();
    assert_eq!(&*calls.lock().unwrap(), &["wake"]);
}

#[derive(Debug, Default)]
struct Owner {
    calls: Arc<Mutex<Vec<&'static str>>>,
}

impl PrivateLifecycleOwner for Owner {
    type Wake = ();
    type Shutdown = ();
    type Escalation = ();
    type Error = ();

    fn wake(&mut self) -> Result<Self::Wake, Self::Error> {
        self.calls.lock().unwrap().push("wake");
        Ok(())
    }

    fn request_shutdown(&mut self) -> Result<Self::Shutdown, Self::Error> {
        Ok(())
    }

    fn escalate_shutdown(&mut self) -> Result<Self::Escalation, Self::Error> {
        Ok(())
    }

    fn needs_drive(&self) -> bool {
        false
    }
}

fn router(
    provider: AgentChatProvider,
    claude: Box<dyn OrdinaryLifecycleHost>,
    codex: Box<dyn OrdinaryLifecycleHost>,
) -> OrdinaryPublicLifecycleRouter<Ledger> {
    OrdinaryPublicLifecycleRouter::new(
        AgentChatReadService::new(Ledger { provider }),
        vec![claude, codex],
    )
    .unwrap()
}

fn host(
    provider: AgentChatProvider,
    calls: Arc<Mutex<Vec<&'static str>>>,
) -> Box<dyn OrdinaryLifecycleHost> {
    Box::new(Host { provider, calls })
}

fn wake() -> PromptWake {
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
