//! Shared fixture for the prompt-provider-provision whole-profile proof (Phase 2).
//!
//! Builds a real `RuntimeFacade` composed with the real `PromptProviderProvisionBoundary` and
//! digest-bound `PrivateProviderProvisioner`, backed by a real on-disk `SqliteLedger` and a real
//! signed ordinary-authority release fixture. Only the outermost npm/binary boundary (installer,
//! post-install verifier) is a controlled test double.

use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
};

use ed25519_dalek::{SigningKey, VerifyingKey};
use gent_drivers::installer::{DependencyInstaller, InstallerError, NpmGlobalPrefix};
use gent_ports::{
    AgentChatPromptLedger, AgentChatWorkspaceLedger, ApprovedPackageInstall, PackageInstallPolicy,
    PackageInstallPolicyError,
};
use gent_protocol::{DependencyProvider, ProviderReadinessFrame};
use gent_runtime::catalog::{RuntimeCapabilityFeature, RuntimeCapabilityProfile};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatProvider,
    AgentChatRequestId, AgentChatRunId, AgentChatSelection, HostEpoch, ReceiptId, WorkspaceRecord,
};
use sha2::Digest;

use crate::{
    authority_clock::AuthorityClock,
    compatibility_assessment::CompatibilityAssessment,
    dependency_catalog::DependencyCatalog,
    ordinary_authority_release::fixture,
    private_provider_provisioning::{
        PrivateProviderProvisioner, ProvisionedProviderLock, ProvisionedProviderVerifier,
        ReleaseAuthorityConfig,
    },
    private_provider_provisioning_sqlite::SqliteProvisionReceiptReader,
    private_provider_review::install_review,
    prompt_provider_provision_boundary::{
        PromptProviderProvisionBoundary, PromptProviderProvisionPort,
    },
    provider_readiness_boundary::ProviderReadinessPort,
    runtime_facade::{DaemonCompositionState, RuntimeFacade},
};

#[derive(Clone)]
pub(super) struct Policy;

impl PackageInstallPolicy for Policy {
    fn approved_package(
        &self,
        provider: &str,
        _: u64,
    ) -> Result<ApprovedPackageInstall, PackageInstallPolicyError> {
        Ok(ApprovedPackageInstall {
            provider: provider.into(),
            package_name: "package".into(),
            version: "1.0.0".into(),
            integrity: "sha512-test".into(),
            package_policy_digest_sha256: "a".repeat(64),
        })
    }
}

#[derive(Clone, Copy)]
pub(super) struct FixedClock(pub(super) u64);

impl AuthorityClock for FixedClock {
    fn now_unix_seconds(&self) -> u64 {
        self.0
    }
}

#[derive(Clone, Default)]
pub(super) struct CountingInstaller(pub(super) Arc<Mutex<u32>>);

impl DependencyInstaller for CountingInstaller {
    fn install(
        &self,
        npm: &NpmGlobalPrefix,
        package: &ApprovedPackageInstall,
    ) -> Result<(), InstallerError> {
        *self.0.lock().unwrap() += 1;
        let bin = npm.prefix().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(&package.provider), "installed provider").unwrap();
        Ok(())
    }
}

#[derive(Clone)]
pub(super) struct FakeVerifier;

impl ProvisionedProviderVerifier for FakeVerifier {
    fn lock(
        &self,
        provider: DependencyProvider,
        prefix: &Path,
    ) -> Result<ProvisionedProviderLock, String> {
        gent_drivers::lock::capture(
            provider.as_str(),
            &prefix.join("bin").join(provider.as_str()),
            "1.0.0",
            "test",
        )
        .map(|run_lock| ProvisionedProviderLock { run_lock })
        .map_err(|error| error.to_string())
    }
}

#[derive(Clone)]
pub(super) struct AmbiguousVerifier;

impl ProvisionedProviderVerifier for AmbiguousVerifier {
    fn lock(&self, _: DependencyProvider, _: &Path) -> Result<ProvisionedProviderLock, String> {
        Err("post-install verification observed an unsupported executable".into())
    }
}

#[derive(Clone)]
pub(super) struct AllowReadiness;

impl ProviderReadinessPort for AllowReadiness {
    fn assess(&self, frame: ProviderReadinessFrame) -> Result<ProviderReadinessFrame, String> {
        Ok(frame)
    }
}

pub(super) const CLOCK: FixedClock = FixedClock(10);

pub(super) fn plan_digest() -> String {
    install_review(
        &DependencyCatalog::default(),
        &Policy,
        &CLOCK,
        DependencyProvider::Codex,
    )
    .unwrap()
    .reviewed_plan_digest
}

pub(super) fn profile() -> RuntimeCapabilityProfile {
    RuntimeCapabilityProfile::new([
        RuntimeCapabilityFeature::AgentChat,
        RuntimeCapabilityFeature::ProviderReadiness,
        RuntimeCapabilityFeature::PromptProviderProvision,
    ])
}

/// A fully composed, digest-bound whole profile: real ledger, real boundary, real provisioner.
pub(super) struct Profile {
    pub(super) runtime: RuntimeFacade,
    pub(super) ledger: SqliteLedger,
    pub(super) saved: AgentChatPromptSaved,
    pub(super) installer_calls: Arc<Mutex<u32>>,
}

pub(super) fn compose<V: ProvisionedProviderVerifier + 'static>(
    data_dir: &Path,
    verifier: V,
) -> Profile {
    let node_runtime = fixture::runtime(&data_dir.join("node"));
    let signer = SigningKey::from_bytes(&[42; 32]);
    let envelope = fixture::release(&signer, node_runtime.node_digest_sha256());
    let release_path = data_dir.join("ordinary-authority.json");
    std::fs::write(&release_path, serde_json::to_vec(&envelope).unwrap()).unwrap();
    let release_digest = hex::encode(sha2::Sha256::digest(
        serde_json::to_vec(&serde_json::to_value(&envelope).unwrap()).unwrap(),
    ));
    let root_keys: BTreeMap<String, VerifyingKey> = fixture::root_keys(&signer);

    let installer_calls = Arc::new(Mutex::new(0));
    let installer = CountingInstaller(Arc::clone(&installer_calls));

    let gentd_dir = data_dir.join("gentd");
    std::fs::create_dir_all(&gentd_dir).unwrap();
    let state =
        DaemonCompositionState::open(&gentd_dir, &profile(), CompatibilityAssessment::default())
            .unwrap();
    let ledger = state.ledger().clone();
    let receipts = SqliteProvisionReceiptReader::new(ledger.clone());

    let provisioner = PrivateProviderProvisioner::with_release_authority(
        node_runtime,
        installer,
        Policy,
        Some(verifier),
        receipts,
        ReleaseAuthorityConfig {
            path: release_path,
            root_keys,
        },
    );

    let boundary = PromptProviderProvisionBoundary::new(
        ledger.clone(),
        DependencyCatalog::default(),
        Policy,
        provisioner,
        CLOCK,
        release_digest,
    );
    let authority: Arc<dyn PromptProviderProvisionPort> = Arc::new(boundary);

    let saved = seed(&ledger);

    let runtime = RuntimeFacade::from_state_with_prompt_provider_provision_authority(
        state,
        None,
        Arc::new(AllowReadiness),
        authority,
    )
    .unwrap();

    Profile {
        runtime,
        ledger,
        saved,
        installer_calls,
    }
}

fn seed(ledger: &SqliteLedger) -> AgentChatPromptSaved {
    let conversation = AgentChatConversationCreate {
        receipt_id: ReceiptId("conversation-receipt".into()),
        idempotency_key: "conversation-key".into(),
        host_epoch: HostEpoch(1),
        conversation_id: AgentChatConversationId("conversation".into()),
        run_id: AgentChatRunId("run".into()),
        selection: AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "gpt-5.6".into(),
            effort: AgentChatEffort::High,
            mode: AgentChatMode::Agent,
        },
    };
    ledger
        .create_agent_chat_conversation_in_workspace(
            &conversation,
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-request".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: conversation.conversation_id,
            disposition: AgentChatPromptDisposition::Send,
            text: "install then send".into(),
        })
        .unwrap()
}
