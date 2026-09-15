use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use ed25519_dalek::SigningKey;
use gent_ports::{
    AgentChatPromptLedger, AgentChatReadLedger, AgentChatWorkspaceLedger,
    ConversationActivityLedger, ConversationLedger, Ledger, LedgerError,
    ProvisionedProviderLockReader,
};
use gent_protocol::{DependencyProvider, ProviderReadinessFrame, ProviderReadinessReviewState};
use gent_runtime::catalog::RuntimeCapabilityProfile;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatProvider,
    AgentChatRequestId, AgentChatRunId, AgentChatSelection, ConversationActivityFact,
    DurableTurnPhase, NormalizedTranscriptKind, PromptHoldReason, ProviderInstallProvenance,
    ProvisionedProviderInstallation, ProvisionedProviderLock, ReceiptId, RunVersionLock,
    WorkspaceRecord,
};
use serde_json::Value;

use super::{StandaloneAuthorityConfig, compose_standalone_authority};
use crate::{
    CompatibilityAssessment,
    agent_chat_api::{PromptCommitWake, PromptWake},
    ordinary_authority_release::fixture,
    provider_executables::ProviderExecutables,
    runtime_facade::DaemonCompositionState,
    standalone_authority_composition::StandaloneAuthorityRuntime,
    standalone_authority_release::StandaloneAuthorityRelease,
    standalone_provider_setup::{provider_executable, provider_prefix},
};

const FAKE_CODEX: &str = include_str!("../testdata/fake-codex-app-server.py");
const FAKE_CLAUDE: &str = include_str!("../testdata/fake-claude-stream-json.py");

#[derive(Clone)]
struct Installed(Arc<Mutex<Option<RunVersionLock>>>);

impl ProvisionedProviderLockReader for Installed {
    fn find_provisioned_provider_installation(
        &self,
        _: &str,
    ) -> Result<Option<ProvisionedProviderInstallation>, LedgerError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .clone()
            .map(|run_lock| ProvisionedProviderInstallation {
                lock: ProvisionedProviderLock { run_lock },
                provenance: ProviderInstallProvenance {
                    package_name: "package".into(),
                    package_version: "0.1.0".into(),
                    package_integrity: "sha512-test".into(),
                    package_policy_digest_sha256: "a".repeat(64),
                    node_runtime_digest_sha256: "b".repeat(64),
                    release_artifact_digest_sha256: "c".repeat(64),
                    receipt_fingerprint_sha256: "d".repeat(64),
                },
            }))
    }
}

struct InstalledProvider {
    provider: AgentChatProvider,
    directory: tempfile::TempDir,
    signer: SigningKey,
    release: StandaloneAuthorityRelease,
    installed: Installed,
    state: DaemonCompositionState,
    runtime: StandaloneAuthorityRuntime,
    prompts: u32,
}

fn boot(
    provider_root: &std::path::Path,
    installed: &Installed,
    release: &StandaloneAuthorityRelease,
) -> (DaemonCompositionState, StandaloneAuthorityRuntime) {
    let data_dir = provider_root.join("data");
    let state = DaemonCompositionState::open(
        &data_dir,
        &RuntimeCapabilityProfile::default(),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    state.fence_unclean_predecessor().unwrap();
    let runtime = compose_standalone_authority(
        &state,
        &StandaloneAuthorityConfig {
            data_dir: data_dir.clone(),
            executables: ProviderExecutables::standalone(
                None,
                None,
                &data_dir,
                installed.clone(),
                Some(release.clone()),
            ),
            mcp_config: None,
        },
    )
    .unwrap();
    runtime
        .router()
        .lock()
        .unwrap()
        .activate_recovery()
        .unwrap();
    (state, runtime)
}

impl InstalledProvider {
    fn start(provider: AgentChatProvider) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let signer = SigningKey::from_bytes(&[33; 32]);
        let release = StandaloneAuthorityRelease::configured(
            directory.path().join("authority.json"),
            &[format!(
                "root:{}",
                hex::encode(signer.verifying_key().as_bytes())
            )],
            fixture::runtime(&directory.path().join("runtime")),
        )
        .unwrap();
        let installed = Installed(Arc::new(Mutex::new(None)));
        fs::create_dir_all(directory.path().join("data")).unwrap();
        fs::create_dir_all(directory.path().join("workspace")).unwrap();
        let (state, runtime) = boot(directory.path(), &installed, &release);
        let installed_provider = Self {
            provider,
            directory,
            signer,
            release,
            installed,
            state,
            runtime,
            prompts: 0,
        };
        installed_provider.install("1.0.0");
        installed_provider
    }

    fn restart(&mut self) {
        let (state, runtime) = boot(self.directory.path(), &self.installed, &self.release);
        self.runtime = runtime;
        self.state = state;
    }

    fn name(&self) -> &'static str {
        match self.provider {
            AgentChatProvider::Claude => "claude",
            _ => "codex",
        }
    }

    fn binary(&self) -> PathBuf {
        let dependency = match self.provider {
            AgentChatProvider::Claude => DependencyProvider::Claude,
            _ => DependencyProvider::Codex,
        };
        provider_executable(
            &provider_prefix(&self.directory.path().join("data")),
            dependency,
        )
        .unwrap()
    }

    fn script(&self, version: &str) -> String {
        let script = match self.provider {
            AgentChatProvider::Claude => FAKE_CLAUDE,
            _ => FAKE_CODEX,
        };
        format!("{script}\nVERSION = \"{version}\"\n")
    }

    fn sign_unreleased(&self, version: &str) {
        use sha2::Digest;
        self.sign(
            version,
            &hex::encode(sha2::Sha256::digest(self.script(version))),
        );
    }

    fn write_version(&self, version: &str) -> RunVersionLock {
        let binary = self.binary();
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, self.script(version)).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
        gent_drivers::lock::capture(
            self.name(),
            &binary,
            &format!("{} {version}", self.name()),
            &format!("{}-0.1.0", self.name()),
        )
        .unwrap()
    }

    fn sign(&self, version: &str, digest: &str) {
        let node = self
            .release
            .runtime()
            .unwrap()
            .node_digest_sha256()
            .to_owned();
        let signed = fixture::release_for_provider(
            &self.signer,
            &node,
            self.name(),
            &format!("{} {version}", self.name()),
            digest,
        );
        fs::write(
            self.directory.path().join("authority.json"),
            serde_json::to_vec(&signed).unwrap(),
        )
        .unwrap();
    }

    fn install(&self, version: &str) -> RunVersionLock {
        let lock = self.write_version(version);
        self.sign(version, &lock.digest_sha256);
        *self.installed.0.lock().unwrap() = Some(lock.clone());
        lock
    }

    fn conversation(&self) -> AgentChatConversationId {
        let conversation_id = AgentChatConversationId("conversation-installed".into());
        self.state
            .ledger()
            .create_agent_chat_conversation_in_workspace(
                &AgentChatConversationCreate {
                    receipt_id: ReceiptId("create-installed".into()),
                    idempotency_key: "create-installed".into(),
                    host_epoch: self.epoch(),
                    conversation_id: conversation_id.clone(),
                    run_id: AgentChatRunId("run-installed".into()),
                    selection: AgentChatSelection {
                        provider: self.provider,
                        model: match self.provider {
                            AgentChatProvider::Claude => "sonnet".into(),
                            _ => "gpt-5.6-luna".into(),
                        },
                        effort: AgentChatEffort::Low,
                        mode: AgentChatMode::Agent,
                    },
                },
                &WorkspaceRecord {
                    workspace_id: "workspace-installed".into(),
                    canonical_path: fs::canonicalize(self.directory.path().join("workspace"))
                        .unwrap()
                        .display()
                        .to_string(),
                },
            )
            .unwrap();
        conversation_id
    }

    fn epoch(&self) -> gent_types::HostEpoch {
        self.state.coordinator().status().unwrap().host_epoch
    }

    fn save(
        &mut self,
        conversation_id: &AgentChatConversationId,
        text: &str,
    ) -> AgentChatPromptSaved {
        self.prompts += 1;
        self.state
            .ledger()
            .save_agent_chat_prompt(&AgentChatPromptCreate {
                request_id: AgentChatRequestId(format!("request-{}", self.prompts)),
                receipt_id: ReceiptId(format!("receipt-{}", self.prompts)),
                host_epoch: self.epoch(),
                conversation_id: conversation_id.clone(),
                disposition: AgentChatPromptDisposition::Send,
                text: text.into(),
                attachment_ids: vec![],
                tool_source_ids: vec![],
            })
            .unwrap()
    }

    fn send(
        &mut self,
        conversation_id: &AgentChatConversationId,
        text: &str,
    ) -> AgentChatPromptSaved {
        let saved = self.save(conversation_id, text);
        self.wake(&saved);
        saved
    }

    fn wake(&self, saved: &AgentChatPromptSaved) {
        self.runtime
            .prompt_ingress()
            .wake_after_prompt_commit(PromptWake {
                conversation_id: AgentChatConversationId(saved.message.conversation_id.clone()),
                run_id: saved.run_id.clone(),
                receipt_id: saved.receipt.receipt_id.clone(),
                disposition: AgentChatPromptDisposition::Send,
            })
            .unwrap();
    }

    fn settle(&self, saved: &AgentChatPromptSaved) -> DurableTurnPhase {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self.phase(saved).is_terminal() {
            self.runtime.drive_once().unwrap();
            assert!(
                Instant::now() < deadline,
                "timed out waiting for the installed provider turn"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        self.phase(saved)
    }

    fn drive_briefly(&self) {
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            self.runtime.drive_once().unwrap();
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn phase(&self, saved: &AgentChatPromptSaved) -> DurableTurnPhase {
        self.state
            .ledger()
            .find_turn(&saved.message.turn_id)
            .unwrap()
            .unwrap()
            .phase
    }

    fn transcript(
        &self,
        saved: &AgentChatPromptSaved,
        kind: NormalizedTranscriptKind,
    ) -> Vec<String> {
        let mut texts = Vec::new();
        let mut after = None;
        loop {
            let page = self
                .state
                .ledger()
                .read_agent_chat_transcript(&saved.message.conversation_id, after, 100)
                .unwrap();
            texts.extend(
                page.events
                    .into_iter()
                    .filter(|event| {
                        event.turn_id == saved.message.turn_id
                            && event.kind == kind
                            && (!event.is_partial
                                || kind == NormalizedTranscriptKind::AssistantMessage)
                    })
                    .map(|event| event.text),
            );
            match page.next_after_cursor {
                Some(next) => after = Some(next),
                None => return texts,
            }
        }
    }

    fn reply(&self, saved: &AgentChatPromptSaved) -> String {
        self.transcript(saved, NormalizedTranscriptKind::AssistantMessage)
            .concat()
    }

    fn requests(&self, method: &str) -> Vec<Value> {
        fs::read_to_string(self.binary().with_file_name("requests.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|request| request["method"] == method)
            .collect()
    }

    fn lose_thread(&self, thread: &str) {
        fs::remove_file(
            self.binary()
                .with_file_name("threads")
                .join(format!("{thread}.jsonl")),
        )
        .unwrap();
    }

    fn claude_launches(&self) -> Vec<Vec<String>> {
        fs::read_to_string(self.binary().with_file_name("launches.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn launched_digest(&self, saved: &AgentChatPromptSaved) -> String {
        self.state
            .ledger()
            .find_run_version_lock(&saved.run_id.0)
            .unwrap()
            .unwrap()
            .digest_sha256
    }

    fn bound_session(&self, saved: &AgentChatPromptSaved) -> String {
        self.state
            .ledger()
            .find_run_session_binding(&saved.run_id.0)
            .unwrap()
            .unwrap()
            .provider_session_id
    }

    fn events(&self, kind: &str) -> Vec<Value> {
        self.state
            .ledger()
            .read_event_page(0, 1000)
            .unwrap()
            .events
            .into_iter()
            .filter(|event| event.kind == kind)
            .map(|event| event.payload)
            .collect()
    }

    fn held_for_install(&self, saved: &AgentChatPromptSaved) -> bool {
        self.state
            .ledger()
            .read_conversation_activity_page(&saved.message.conversation_id, &saved.run_id.0, 0, 50)
            .unwrap()
            .facts
            .iter()
            .any(|fact| matches!(fact, ConversationActivityFact::PromptHeld { message_id, reason: PromptHoldReason::ProviderInstall, .. } if *message_id == saved.message.message_id))
    }

    fn readiness(&self, saved: &AgentChatPromptSaved) -> ProviderReadinessFrame {
        self.runtime
            .provider_readiness_port()
            .assess(ProviderReadinessFrame::Assess {
                conversation_id: AgentChatConversationId(saved.message.conversation_id.clone()),
                run_id: saved.run_id.clone(),
            })
            .unwrap()
    }

    fn assert_reinstall_review(&self, saved: &AgentChatPromptSaved) {
        self.drive_briefly();
        assert!(self.held_for_install(saved));
        assert!(!self.phase(saved).is_terminal());
        let ProviderReadinessFrame::Review { state, .. } = self.readiness(saved) else {
            panic!("an unauthorized installed binary must offer a reinstall review");
        };
        assert_eq!(state, ProviderReadinessReviewState::InvalidInstallation);
    }
}

#[test]
fn an_installed_codex_launches_its_native_binary_and_a_tampered_one_is_held_for_reinstall() {
    let mut codex = InstalledProvider::start(AgentChatProvider::Codex);
    let conversation = codex.conversation();
    let first = codex.send(&conversation, "installed prompt");
    assert_eq!(codex.settle(&first), DurableTurnPhase::Completed);
    let launches = codex.requests("launch");
    assert_eq!(launches.len(), 1);
    assert_eq!(
        launches[0]["params"]["argv"],
        serde_json::json!(["app-server", "-c", "check_for_update_on_startup=false"])
    );
    assert_eq!(launches[0]["params"]["managed"], serde_json::json!([]));

    let binary = codex.binary();
    let original = fs::read(&binary).unwrap();
    fs::write(&binary, [original.as_slice(), b"\n"].concat()).unwrap();
    let tampered = codex.send(&conversation, "tampered prompt");
    codex.assert_reinstall_review(&tampered);
    assert_eq!(
        codex.requests("launch").len(),
        1,
        "the tampered binary never started"
    );

    codex.install("1.0.1");
    codex.wake(&tampered);
    assert_eq!(codex.settle(&tampered), DurableTurnPhase::Completed);
    assert!(matches!(
        codex.readiness(&tampered),
        ProviderReadinessFrame::Ready { .. }
    ));
}

#[path = "standalone_authority_provider_upgrade_tests.rs"]
mod upgrade;

#[path = "standalone_authority_installed_steer_tests.rs"]
mod steer;
