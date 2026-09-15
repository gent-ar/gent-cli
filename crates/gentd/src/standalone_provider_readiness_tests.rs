use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use ed25519_dalek::SigningKey;
use gent_ports::{AgentChatPromptDispatchLedger, AgentChatPromptLedger, AgentChatWorkspaceLedger};
use gent_protocol::{
    ProviderReadinessFrame, ProviderReadinessReviewState, ProviderReadinessUnavailable,
};
use gent_runtime::AgentChatReadService;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, HostEpoch, ReceiptId, WorkspaceRecord,
};

use super::{StandaloneProviderReadinessAuthority, StandalonePublicProviderReadiness};
use crate::{
    agent_chat_api::{PromptCommitWake, PromptWake},
    ordinary_lifecycle_cadence::pair_with_standalone_models,
    ordinary_lifecycle_router::{OrdinaryLifecycleHost, OrdinaryPublicLifecycleRouter},
    provider_executables::ProviderExecutables,
    provider_readiness_boundary::ProviderReadinessPort,
    standalone_authority_release::StandaloneAuthorityRelease,
};

#[derive(Debug)]
struct Unready;

impl StandalonePublicProviderReadiness for Unready {
    fn is_ready(&self, _: AgentChatProvider) -> Result<bool, String> {
        Ok(false)
    }
}

struct Host {
    woke: Arc<AtomicBool>,
}

impl OrdinaryLifecycleHost for Host {
    fn provider(&self) -> AgentChatProvider {
        AgentChatProvider::Claude
    }

    fn arm_authority_recovery(&mut self) -> Result<(), ()> {
        Ok(())
    }

    fn wake(&mut self) -> Result<(), ()> {
        self.woke.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn drive(&mut self) -> Result<(), ()> {
        Ok(())
    }

    fn needs_drive(&self) -> bool {
        false
    }
}

#[test]
fn explicit_provider_executable_is_ready_only_for_the_exact_current_run() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("claude");
    std::fs::write(&executable, "ready").unwrap();
    let ledger = conversation(directory.path(), AgentChatProvider::Claude);
    let authority = StandaloneProviderReadinessAuthority::new(
        ledger.clone(),
        ProviderExecutables::standalone(
            Some(executable),
            Some(directory.path().join("codex")),
            directory.path(),
            ledger,
            None,
        ),
        crate::local_model_jobs::tests::models(directory.path()),
    );

    assert_eq!(
        authority.assess(request("run")).unwrap(),
        ProviderReadinessFrame::Ready {
            conversation_id: AgentChatConversationId("conversation".into()),
            run_id: AgentChatRunId("run".into()),
            provider: AgentChatProvider::Claude,
        }
    );
    assert!(authority.assess(request("other-run")).is_err());
}

#[test]
fn unproven_public_provider_is_unavailable_without_starting_or_installing_it() {
    let directory = tempfile::tempdir().unwrap();
    let ledger = conversation(directory.path(), AgentChatProvider::Codex);
    let authority = StandaloneProviderReadinessAuthority::new(
        ledger.clone(),
        ProviderExecutables::standalone(None, None, directory.path(), ledger, None),
        crate::local_model_jobs::tests::models(directory.path()),
    );

    assert_eq!(
        authority.assess(request("run")).unwrap(),
        ProviderReadinessFrame::Unavailable {
            conversation_id: AgentChatConversationId("conversation".into()),
            run_id: AgentChatRunId("run".into()),
            reason: ProviderReadinessUnavailable::ProvenanceUnreadable,
        }
    );
}

#[tokio::test]
async fn a_gent_run_reports_its_local_model_hold_until_the_model_is_verified() {
    let directory = tempfile::tempdir().unwrap();
    let ledger = conversation_with_model(
        directory.path(),
        AgentChatProvider::Claurst,
        "qwen3-8b-q4-k-m",
    );
    let models = crate::local_model_jobs::tests::models(directory.path())
        .with_download_transport(Arc::new(crate::local_model_jobs::tests::BlockingTransport));
    let authority = StandaloneProviderReadinessAuthority::new(
        ledger.clone(),
        ProviderExecutables::standalone(None, None, directory.path(), ledger, None),
        models.clone(),
    );
    let plan = models.provisioner.plan("qwen3-8b-q4-k-m").unwrap();
    let local = |install| ProviderReadinessFrame::LocalModel {
        conversation_id: AgentChatConversationId("conversation".into()),
        run_id: AgentChatRunId("run".into()),
        model_id: "qwen3-8b-q4-k-m".into(),
        install,
    };

    assert_eq!(
        authority.assess(request("run")).unwrap(),
        local(gent_protocol::LocalModelInstallState::NotInstalled)
    );
    models
        .downloads
        .start(
            "qwen3-8b-q4-k-m",
            crate::local_model_jobs::DownloadHolder::Background,
        )
        .unwrap();
    assert_eq!(
        authority.assess(request("run")).unwrap(),
        local(gent_protocol::LocalModelInstallState::Downloading {
            downloaded_bytes: 0,
            total_bytes: plan.expected_bytes,
        })
    );
    models.downloads.cancel("qwen3-8b-q4-k-m").unwrap();
    models.provisioner.ensure_storage(&plan).unwrap();
    std::fs::File::create(&plan.destination)
        .unwrap()
        .set_len(plan.expected_bytes)
        .unwrap();
    crate::local_model_integrity::remember_sha256(&plan.destination, &plan.expected_sha256);
    assert_eq!(
        authority.assess(request("run")).unwrap(),
        ProviderReadinessFrame::Ready {
            conversation_id: AgentChatConversationId("conversation".into()),
            run_id: AgentChatRunId("run".into()),
            provider: AgentChatProvider::Claurst,
        }
    );
}

#[test]
fn signed_release_produces_a_daemon_owned_missing_install_review() {
    let directory = tempfile::tempdir().unwrap();
    let runtime =
        crate::ordinary_authority_release::fixture::runtime(&directory.path().join("runtime"));
    let signer = SigningKey::from_bytes(&[9; 32]);
    let envelope =
        crate::ordinary_authority_release::fixture::release(&signer, runtime.node_digest_sha256());
    let path = directory.path().join("authority.json");
    std::fs::write(&path, serde_json::to_vec(&envelope).unwrap()).unwrap();
    let release = StandaloneAuthorityRelease::configured(
        path,
        &[format!(
            "root:{}",
            hex::encode(signer.verifying_key().as_bytes())
        )],
        runtime,
    )
    .unwrap();
    let ledger = conversation(directory.path(), AgentChatProvider::Codex);
    let authority = StandaloneProviderReadinessAuthority::new(
        ledger.clone(),
        ProviderExecutables::standalone(None, None, directory.path(), ledger, Some(release)),
        crate::local_model_jobs::tests::models(directory.path()),
    );

    let ProviderReadinessFrame::Review { state, review, .. } =
        authority.assess(request("run")).unwrap()
    else {
        panic!("expected signed install review");
    };
    assert_eq!(state, ProviderReadinessReviewState::MissingInstall);
    assert_eq!(review.package.package_name, "@openai/codex");
    assert!(review.consent_required);
}

#[test]
fn a_release_that_does_not_approve_gents_node_reports_an_unverified_runtime_instead_of_a_review() {
    let directory = tempfile::tempdir().unwrap();
    let runtime =
        crate::ordinary_authority_release::fixture::runtime(&directory.path().join("runtime"));
    let signer = SigningKey::from_bytes(&[9; 32]);
    let envelope = crate::ordinary_authority_release::fixture::release(&signer, &"e".repeat(64));
    let path = directory.path().join("authority.json");
    std::fs::write(&path, serde_json::to_vec(&envelope).unwrap()).unwrap();
    let release = StandaloneAuthorityRelease::configured(
        path,
        &[format!(
            "root:{}",
            hex::encode(signer.verifying_key().as_bytes())
        )],
        runtime,
    )
    .unwrap();
    let ledger = conversation(directory.path(), AgentChatProvider::Codex);
    let authority = StandaloneProviderReadinessAuthority::new(
        ledger.clone(),
        ProviderExecutables::standalone(None, None, directory.path(), ledger, Some(release)),
        crate::local_model_jobs::tests::models(directory.path()),
    );

    assert!(!authority.is_ready(AgentChatProvider::Codex).unwrap());
    assert_eq!(
        authority.assess(request("run")).unwrap(),
        ProviderReadinessFrame::Unavailable {
            conversation_id: AgentChatConversationId("conversation".into()),
            run_id: AgentChatRunId("run".into()),
            reason: ProviderReadinessUnavailable::RuntimeUnverified,
        }
    );
}

#[derive(Clone)]
struct InstalledCodex(gent_types::RunVersionLock);

impl gent_ports::ProvisionedProviderLockReader for InstalledCodex {
    fn find_provisioned_provider_installation(
        &self,
        _: &str,
    ) -> Result<Option<gent_types::ProvisionedProviderInstallation>, gent_ports::LedgerError> {
        Ok(Some(gent_types::ProvisionedProviderInstallation {
            lock: gent_types::ProvisionedProviderLock {
                run_lock: self.0.clone(),
            },
            provenance: gent_types::ProviderInstallProvenance {
                package_name: "@openai/codex".into(),
                package_version: "0.1.0-darwin-arm64".into(),
                package_integrity: "sha512-test".into(),
                package_policy_digest_sha256: "a".repeat(64),
                node_runtime_digest_sha256: "b".repeat(64),
                release_artifact_digest_sha256: "c".repeat(64),
                receipt_fingerprint_sha256: "d".repeat(64),
            },
        }))
    }
}

#[test]
fn an_installed_binary_the_signed_release_does_not_authorize_is_offered_a_reinstall_review() {
    let directory = tempfile::tempdir().unwrap();
    let runtime =
        crate::ordinary_authority_release::fixture::runtime(&directory.path().join("runtime"));
    let signer = SigningKey::from_bytes(&[9; 32]);
    let envelope =
        crate::ordinary_authority_release::fixture::release(&signer, runtime.node_digest_sha256());
    let path = directory.path().join("authority.json");
    std::fs::write(&path, serde_json::to_vec(&envelope).unwrap()).unwrap();
    let release = StandaloneAuthorityRelease::configured(
        path,
        &[format!(
            "root:{}",
            hex::encode(signer.verifying_key().as_bytes())
        )],
        runtime,
    )
    .unwrap();
    let codex = crate::standalone_provider_setup::provider_executable(
        &crate::standalone_provider_setup::provider_prefix(directory.path()),
        gent_protocol::DependencyProvider::Codex,
    )
    .unwrap();
    std::fs::create_dir_all(codex.parent().unwrap()).unwrap();
    std::fs::write(&codex, "tampered codex").unwrap();
    let lock = gent_drivers::lock::capture("codex", &codex, "0.1.0", "codex-0.1.0").unwrap();
    let ledger = conversation(directory.path(), AgentChatProvider::Codex);
    let authority = StandaloneProviderReadinessAuthority::new(
        ledger,
        ProviderExecutables::standalone(
            None,
            None,
            directory.path(),
            InstalledCodex(lock),
            Some(release),
        ),
        crate::local_model_jobs::tests::models(directory.path()),
    );

    assert!(!authority.is_ready(AgentChatProvider::Codex).unwrap());
    let ProviderReadinessFrame::Review { state, review, .. } =
        authority.assess(request("run")).unwrap()
    else {
        panic!("expected a reinstall review for an unauthorized installed binary");
    };
    assert_eq!(state, ProviderReadinessReviewState::InvalidInstallation);
    assert_eq!(review.package.package_name, "@openai/codex");
}

#[test]
fn unready_public_provider_keeps_the_prompt_held_and_never_wakes_its_host() {
    let directory = tempfile::tempdir().unwrap();
    let ledger = conversation(directory.path(), AgentChatProvider::Claude);
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-request".into()),
            receipt_id: ReceiptId("prompt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation".into()),
            disposition: AgentChatPromptDisposition::Send,
            text: "continue".into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    let woke = Arc::new(AtomicBool::new(false));
    let router = Arc::new(Mutex::new(
        OrdinaryPublicLifecycleRouter::new(
            AgentChatReadService::new(ledger.clone()),
            vec![Box::new(Host {
                woke: Arc::clone(&woke),
            })],
        )
        .unwrap(),
    ));
    let (_, mut ingress, _) = pair_with_standalone_models(
        router,
        ledger.clone(),
        HostEpoch(1),
        crate::local_model_jobs::tests::models(directory.path()),
        Arc::new(Unready),
    );

    ingress
        .wake_after_prompt_commit(PromptWake {
            conversation_id: AgentChatConversationId("conversation".into()),
            run_id: saved.run_id,
            receipt_id: saved.receipt.receipt_id,
            disposition: AgentChatPromptDisposition::Send,
        })
        .unwrap();

    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch("daemon", HostEpoch(1), AgentChatProvider::Claude)
            .unwrap()
            .is_none()
    );
    assert!(!woke.load(Ordering::SeqCst));
}

fn request(run_id: &str) -> ProviderReadinessFrame {
    ProviderReadinessFrame::Assess {
        conversation_id: AgentChatConversationId("conversation".into()),
        run_id: AgentChatRunId(run_id.into()),
    }
}

fn conversation(path: &std::path::Path, provider: AgentChatProvider) -> SqliteLedger {
    conversation_with_model(path, provider, "model")
}

fn conversation_with_model(
    path: &std::path::Path,
    provider: AgentChatProvider,
    model: &str,
) -> SqliteLedger {
    let ledger = SqliteLedger::open(path.join("gent.db")).unwrap();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create".into()),
                idempotency_key: "create".into(),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation".into()),
                run_id: AgentChatRunId("run".into()),
                selection: AgentChatSelection {
                    provider,
                    model: model.into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Ask,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: path.display().to_string(),
            },
        )
        .unwrap();
    ledger
}
