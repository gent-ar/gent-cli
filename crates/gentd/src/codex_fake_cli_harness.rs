use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use gent_drivers::process::SystemLauncher;
use gent_ports::{
    AgentChatProjectionLedger, AgentChatPromptLedger, AgentChatReadLedger,
    AgentChatWorkspaceLedger, ConversationLedger, Ledger,
};
use gent_runtime::{AgentChatReadService, catalog::RuntimeCapabilityProfile};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProjectionEvent, AgentChatPromptCreate, AgentChatPromptDisposition,
    AgentChatPromptSaved, AgentChatProvider, AgentChatRequestId, AgentChatRunId,
    AgentChatSelection, DurableTurnPhase, HostEpoch, NormalizedTranscriptEvent,
    NormalizedTranscriptKind, ReceiptId, WorkspaceRecord,
};
use serde_json::Value;

use super::{StandaloneCodexConfig, compose_standalone_codex};
use crate::{
    CompatibilityAssessment,
    agent_chat_api::{PromptCommitWake, PromptWake},
    ordinary_lifecycle_router::{OrdinaryProviderHost, OrdinaryPublicLifecycleRouter},
    provider_lifecycle_host::ProviderLifecycleHost,
    runtime_facade::DaemonCompositionState,
};

const FAKE_CODEX: &str = include_str!("../testdata/fake-codex-app-server.py");

pub(crate) struct FakeCodexDaemon {
    root: tempfile::TempDir,
    workspace: PathBuf,
    pub(crate) ledger: SqliteLedger,
    pub(crate) epoch: HostEpoch,
    pub(crate) router: OrdinaryPublicLifecycleRouter<SqliteLedger>,
    prompts: u32,
}

impl FakeCodexDaemon {
    pub(crate) fn start() -> Self {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("cli/codex");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, FAKE_CODEX).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        fs::create_dir_all(root.path().join("workspace")).unwrap();
        fs::create_dir_all(root.path().join("data")).unwrap();
        let workspace = fs::canonicalize(root.path().join("workspace")).unwrap();
        write_mcp_servers(
            root.path(),
            &serde_json::json!({"docs": {"command": "docs-v1"}}),
        );
        let (ledger, epoch, router) = boot(root.path());
        let mut daemon = Self {
            root,
            workspace,
            ledger,
            epoch,
            router,
            prompts: 0,
        };
        daemon.drive_until("durable recovery", |_| true);
        daemon
    }

    pub(crate) fn restart(&mut self) {
        let (ledger, epoch, router) = boot(self.root.path());
        self.router = router;
        self.ledger = ledger;
        self.epoch = epoch;
        self.drive_until("durable recovery", |_| true);
    }

    pub(crate) fn set_mcp_servers(&self, servers: &Value) {
        write_mcp_servers(self.root.path(), servers);
    }

    pub(crate) fn lose_thread(&self, thread_id: &str) {
        let history = format!("cli/threads/{thread_id}.jsonl");
        fs::remove_file(self.root.path().join(history)).unwrap();
    }

    pub(crate) fn reject_resumes(&self) {
        fs::write(self.root.path().join("cli/resume-rejected"), "").unwrap();
    }

    pub(crate) fn conversation(&self, key: &str) -> (AgentChatConversationId, AgentChatRunId) {
        let conversation_id = AgentChatConversationId(format!("conversation-{key}"));
        let run_id = AgentChatRunId(format!("run-{key}"));
        self.ledger
            .create_agent_chat_conversation_in_workspace(
                &AgentChatConversationCreate {
                    receipt_id: ReceiptId(format!("create-{key}")),
                    idempotency_key: format!("create-{key}"),
                    host_epoch: self.epoch,
                    conversation_id: conversation_id.clone(),
                    run_id: run_id.clone(),
                    selection: AgentChatSelection {
                        provider: AgentChatProvider::Codex,
                        model: "gpt-5.6-luna".into(),
                        effort: AgentChatEffort::Low,
                        mode: AgentChatMode::Agent,
                    },
                },
                &WorkspaceRecord {
                    workspace_id: "workspace-fake-codex".into(),
                    canonical_path: self.workspace.display().to_string(),
                },
            )
            .unwrap();
        (conversation_id, run_id)
    }

    pub(crate) fn prompt(
        &mut self,
        conversation_id: &AgentChatConversationId,
        text: &str,
    ) -> AgentChatPromptSaved {
        self.prompts += 1;
        let disposition = AgentChatPromptDisposition::Send;
        let saved = self
            .ledger
            .save_agent_chat_prompt(&AgentChatPromptCreate {
                request_id: AgentChatRequestId(format!("request-{}", self.prompts)),
                receipt_id: ReceiptId(format!("receipt-{}", self.prompts)),
                host_epoch: self.epoch,
                conversation_id: conversation_id.clone(),
                disposition,
                attachment_ids: vec![],
                tool_source_ids: vec![],
                text: text.into(),
            })
            .unwrap();
        crate::readiness_test_support::release(&self.ledger, &saved);
        self.router
            .wake_after_prompt_commit(PromptWake {
                conversation_id: conversation_id.clone(),
                run_id: saved.run_id.clone(),
                receipt_id: saved.receipt.receipt_id.clone(),
                disposition,
            })
            .unwrap();
        saved
    }

    pub(crate) fn settle(&mut self, prompt: &AgentChatPromptSaved) -> DurableTurnPhase {
        self.drive_until("a settled Codex turn", |daemon| {
            daemon.phase(prompt).is_terminal()
        });
        self.phase(prompt)
    }

    pub(crate) fn drive_until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            self.router
                .drive_once()
                .expect("a run-scoped failure must never escape the lifecycle router");
            if done(self) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub(crate) fn phase(&self, prompt: &AgentChatPromptSaved) -> DurableTurnPhase {
        self.ledger
            .find_turn(&prompt.message.turn_id)
            .unwrap()
            .unwrap()
            .phase
    }

    pub(crate) fn transcript(
        &self,
        prompt: &AgentChatPromptSaved,
    ) -> Vec<NormalizedTranscriptEvent> {
        let mut events = Vec::new();
        let mut after = None;
        loop {
            let page = self
                .ledger
                .read_agent_chat_transcript(&prompt.message.conversation_id, after, 100)
                .unwrap();
            events.extend(
                page.events
                    .into_iter()
                    .filter(|event| event.turn_id == prompt.message.turn_id),
            );
            match page.next_after_cursor {
                Some(next) => after = Some(next),
                None => return events,
            }
        }
    }

    pub(crate) fn reply(&self, prompt: &AgentChatPromptSaved) -> String {
        self.transcript(prompt)
            .into_iter()
            .filter(|event| event.kind == NormalizedTranscriptKind::AssistantMessage)
            .map(|event| event.text)
            .collect()
    }

    pub(crate) fn projection(
        &self,
        conversation_id: &AgentChatConversationId,
    ) -> Vec<AgentChatProjectionEvent> {
        let mut events = Vec::new();
        loop {
            let after = events
                .last()
                .map_or(0, |event: &AgentChatProjectionEvent| event.cursor);
            let page = self
                .ledger
                .agent_chat_projection_page(conversation_id, after, 100)
                .unwrap();
            events.extend(page.events);
            if page.next_after_cursor.is_none() {
                return events;
            }
        }
    }

    pub(crate) fn requests(&self, method: &str) -> Vec<Value> {
        fs::read_to_string(self.root.path().join("cli/requests.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|request| request["method"] == method)
            .collect()
    }

    pub(crate) fn bound_session(&self, run_id: &AgentChatRunId) -> String {
        self.ledger
            .find_run_session_binding(&run_id.0)
            .unwrap()
            .unwrap()
            .provider_session_id
    }
}

fn write_mcp_servers(root: &Path, servers: &Value) {
    let config = serde_json::json!({ "mcpServers": servers });
    fs::write(root.join("mcp.json"), config.to_string()).unwrap();
}

fn boot(
    root: &Path,
) -> (
    SqliteLedger,
    HostEpoch,
    OrdinaryPublicLifecycleRouter<SqliteLedger>,
) {
    let data_dir = root.join("data");
    let state = DaemonCompositionState::open(
        &data_dir,
        &RuntimeCapabilityProfile::default(),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    state.fence_unclean_predecessor().unwrap();
    let epoch = state.coordinator().status().unwrap().host_epoch;
    let host = compose_standalone_codex(
        state.ledger().clone(),
        state.coordinator().clone(),
        &StandaloneCodexConfig {
            data_dir,
            coordinator_id: format!("gentd-{}", epoch.0),
            host_epoch: epoch,
            executables: crate::provider_executables::ProviderExecutables::explicit(
                None,
                Some(root.join("cli/codex")),
            ),
            mcp_servers: None,
            mcp_config: Some(root.join("mcp.json")),
        },
        SystemLauncher::new(64 * 1024),
    )
    .unwrap();
    let mut router = OrdinaryPublicLifecycleRouter::new(
        AgentChatReadService::new(state.ledger().clone()),
        vec![Box::new(OrdinaryProviderHost::new(
            AgentChatProvider::Codex,
            ProviderLifecycleHost::new(host),
        ))],
    )
    .unwrap();
    router.activate_recovery().unwrap();
    (state.ledger().clone(), epoch, router)
}
