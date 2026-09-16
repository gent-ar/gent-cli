use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    time::{Duration, Instant},
};

use gent_drivers::process::SystemLauncher;
use gent_ports::{AgentChatPromptLedger, AgentChatSelectionLedger, AgentChatWorkspaceLedger};
use gent_runtime::{AgentChatReadService, catalog::RuntimeCapabilityProfile};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatProvider,
    AgentChatRequestId, AgentChatRunId, AgentChatSelection, AgentChatSelectionSwitch,
    ContextPolicy, HostEpoch, ReceiptId, WorkspaceRecord,
};

use super::{StandaloneClaudeConfig, compose_standalone_claude};
use crate::{
    CompatibilityAssessment,
    agent_chat_api::{PromptCommitWake, PromptWake},
    ordinary_lifecycle_router::{OrdinaryProviderHost, OrdinaryPublicLifecycleRouter},
    provider_lifecycle_host::ProviderLifecycleHost,
    runtime_facade::DaemonCompositionState,
};

const FAKE_CLAUDE: &str = include_str!("../testdata/fake-claude-stream-json.py");
const BACKGROUND_SUBAGENT: &str =
    include_str!("../../gent-drivers/fixtures/claude-background-subagent.jsonl");

pub(crate) struct FakeClaudeDaemon {
    root: tempfile::TempDir,
    data_dir: PathBuf,
    workspace: PathBuf,
    pub(crate) ledger: SqliteLedger,
    pub(crate) epoch: HostEpoch,
    pub(crate) router: OrdinaryPublicLifecycleRouter<SqliteLedger>,
    prompts: u32,
}

impl FakeClaudeDaemon {
    pub(crate) fn start() -> Self {
        Self::start_with_mcp_servers(None)
    }

    pub(crate) fn start_with_mcp_servers(servers: Option<&str>) -> Self {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("cli/claude");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, FAKE_CLAUDE).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(
            root.path().join("cli/background-subagent.jsonl"),
            BACKGROUND_SUBAGENT,
        )
        .unwrap();
        let workspace = root.path().join("workspace");
        let data_dir = root.path().join("data");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&data_dir).unwrap();
        let state = DaemonCompositionState::open(
            &data_dir,
            &RuntimeCapabilityProfile::default(),
            CompatibilityAssessment::default(),
        )
        .unwrap();
        state.fence_unclean_predecessor().unwrap();
        let epoch = state.coordinator().status().unwrap().host_epoch;
        let mcp_config_path = data_dir.clone();
        let host = compose_standalone_claude(
            state.ledger().clone(),
            state.coordinator().clone(),
            &StandaloneClaudeConfig {
                data_dir,
                coordinator_id: format!("gentd-{}", epoch.0),
                host_epoch: epoch,
                executables: crate::provider_executables::ProviderExecutables::explicit(
                    Some(executable),
                    None,
                ),
                mcp_config: servers.map(|servers| {
                    let path = mcp_config_path.join("standalone-mcp.json");
                    fs::write(&path, servers).unwrap();
                    path
                }),
            },
            SystemLauncher::new(64 * 1024),
        )
        .unwrap();
        let mut router = OrdinaryPublicLifecycleRouter::new(
            AgentChatReadService::new(state.ledger().clone()),
            vec![Box::new(OrdinaryProviderHost::new(
                AgentChatProvider::Claude,
                ProviderLifecycleHost::new(host),
            ))],
        )
        .unwrap();
        router.activate_recovery().unwrap();
        let mut daemon = Self {
            workspace: fs::canonicalize(&workspace).unwrap(),
            data_dir: mcp_config_path,
            root,
            ledger: state.ledger().clone(),
            epoch,
            router,
            prompts: 0,
        };
        daemon.drive_until("durable recovery", |_| true);
        daemon
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
                    selection: selection("sonnet"),
                },
                &WorkspaceRecord {
                    workspace_id: "workspace-fake-claude".into(),
                    canonical_path: self.workspace.display().to_string(),
                },
            )
            .unwrap();
        (conversation_id, run_id)
    }

    pub(crate) fn planning_conversation(
        &self,
        key: &str,
    ) -> (AgentChatConversationId, AgentChatRunId) {
        let (conversation_id, parent_run_id) = self.conversation(key);
        let run_id = AgentChatRunId(format!("run-{key}-plan"));
        self.ledger
            .switch_agent_chat_selection(&AgentChatSelectionSwitch {
                receipt_id: ReceiptId(format!("plan-{key}")),
                idempotency_key: format!("plan-{key}"),
                host_epoch: self.epoch,
                conversation_id: conversation_id.clone(),
                parent_run_id,
                run_id: run_id.clone(),
                selection: AgentChatSelection {
                    mode: AgentChatMode::Plan,
                    ..selection("sonnet")
                },
                context_policy: ContextPolicy::Preserve,
            })
            .unwrap();
        (conversation_id, run_id)
    }

    pub(crate) fn switched_conversation(
        &self,
        key: &str,
    ) -> (AgentChatConversationId, AgentChatRunId) {
        let (conversation_id, parent_run_id) = self.conversation(key);
        let run_id = AgentChatRunId(format!("run-{key}-haiku"));
        self.ledger
            .switch_agent_chat_selection(&AgentChatSelectionSwitch {
                receipt_id: ReceiptId(format!("switch-{key}")),
                idempotency_key: format!("switch-{key}"),
                host_epoch: self.epoch,
                conversation_id: conversation_id.clone(),
                parent_run_id,
                run_id: run_id.clone(),
                selection: selection("haiku"),
                context_policy: ContextPolicy::Preserve,
            })
            .unwrap();
        (conversation_id, run_id)
    }

    pub(crate) fn prompt(
        &mut self,
        conversation_id: &AgentChatConversationId,
        text: &str,
        disposition: AgentChatPromptDisposition,
    ) -> AgentChatPromptSaved {
        self.prompts += 1;
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
}

fn selection(model: &str) -> AgentChatSelection {
    AgentChatSelection {
        provider: AgentChatProvider::Claude,
        model: model.into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Agent,
    }
}

impl FakeClaudeDaemon {
    pub(crate) fn forget_session(&self, session_id: &str) {
        fs::remove_file(
            self.root
                .path()
                .join(format!("cli/sessions/{session_id}.jsonl")),
        )
        .unwrap();
    }

    pub(crate) fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }

    pub(crate) fn fail_resumes_with_an_api_error(&self) {
        fs::write(self.root.path().join("cli/resume-api-error"), "").unwrap();
    }
}

#[path = "claude_fake_cli_harness_reads.rs"]
mod reads;
