use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use ed25519_dalek::{Signer, SigningKey};
use gent_adapters::compatibility::{
    CompatibilityEntry, CompatibilityManifest, SignedCompatibilityManifest, TrustedKeySet,
};
use gent_adapters::compatibility_cache::CachedCompatibilityManifest;
use gent_drivers::SessionEffect;
use gent_ports::{
    AgentChatLedger, AgentChatPromptLedger, ConversationActivityLedger, PublicProviderResolver,
    PublicProviderRunError, PublicProviderRunner,
};
use gent_protocol::{DependencyProvider, PublicRunOutcome, PublicRunStartRequest};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, CapabilitySet, ConversationActivityFact,
    ConversationActivityScope, HostEpoch, NormalizedProviderEvent, NormalizedTranscriptKind,
    ReceiptId, RunVersionLock,
};

use crate::authority_profile::{
    AuthorityProfileConfig, PublicDriverApproval, PublicDriverRequest, ValidatedAuthorityProfile,
};
use crate::compatibility_assessment::CompatibilityAssessment;
use crate::public_driver_runtime::{
    PublicDriverFact, PublicDriverFactResult, PublicDriversRuntime, PublicDriversRuntimeError,
};

#[derive(Debug)]
struct Runner(Arc<AtomicUsize>);

impl PublicProviderRunner for Runner {
    fn start(&self, _: &str, _: &RunVersionLock) -> Result<(), PublicProviderRunError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn resume(&self, _: &str, _: &RunVersionLock, _: &str) -> Result<(), PublicProviderRunError> {
        Ok(())
    }

    fn interrupt(&self, _: &str) -> Result<(), PublicProviderRunError> {
        Ok(())
    }
}

#[derive(Debug)]
struct Resolver;

impl PublicProviderResolver for Resolver {
    fn resolve(&self, provider: &str) -> Result<RunVersionLock, PublicProviderRunError> {
        (provider == "claude")
            .then(lock)
            .ok_or(PublicProviderRunError::CompatibilityDenied)
    }
}

fn lock() -> RunVersionLock {
    RunVersionLock {
        provider: "claude".into(),
        canonical_path: "/verified/claude".into(),
        file_identity: "1:2".into(),
        digest_sha256: "a".repeat(64),
        version: "1.0".into(),
        compatibility_entry: "claude-1".into(),
    }
}

fn compatibility() -> CompatibilityAssessment {
    let key = SigningKey::from_bytes(&[9; 32]);
    let payload = CompatibilityManifest {
        manifest_version: 1,
        expires_at_unix_seconds: 20,
        entries: vec![CompatibilityEntry {
            id: "claude-1".into(),
            provider: "claude".into(),
            version: "1.0".into(),
            digest_sha256: "a".repeat(64),
            revoked: false,
        }],
    };
    let manifest = SignedCompatibilityManifest {
        key_id: "test".into(),
        signature_hex: hex::encode(key.sign(&serde_json::to_vec(&payload).unwrap()).to_bytes()),
        payload,
    };
    let mut keys = TrustedKeySet::default();
    keys.trust("test", key.verifying_key());
    let cached = CachedCompatibilityManifest::verify(manifest, &keys, 1).unwrap();
    CompatibilityAssessment::configured(keys, cached, 10)
}

fn approved(compatibility: &CompatibilityAssessment) -> ValidatedAuthorityProfile {
    AuthorityProfileConfig {
        public_drivers: PublicDriverRequest::Approved(PublicDriverApproval {
            evidence_reference: "approved-evidence".into(),
            compatibility_manifest_sha256: compatibility.manifest_sha256().unwrap(),
        }),
        ..AuthorityProfileConfig::default()
    }
    .validate()
    .unwrap()
}

fn runtime(
    ledger: SqliteLedger,
    profile: ValidatedAuthorityProfile,
    compatibility: CompatibilityAssessment,
) -> (
    PublicDriversRuntime<SqliteLedger, Runner, Resolver>,
    Arc<AtomicUsize>,
) {
    let starts = Arc::new(AtomicUsize::new(0));
    let runtime = PublicDriversRuntime::new(
        profile,
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger,
        compatibility,
        Runner(Arc::clone(&starts)),
        Resolver,
    )
    .unwrap();
    (runtime, starts)
}

#[test]
fn approved_profile_connects_precreated_chat_runs_to_lifecycle_and_activity_ingress() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_agent_chat_conversation(&AgentChatConversationCreate {
            receipt_id: ReceiptId("receipt-conversation".into()),
            idempotency_key: "conversation-key".into(),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            run_id: AgentChatRunId("run-a".into()),
            selection: AgentChatSelection {
                provider: AgentChatProvider::Claude,
                model: "claude".into(),
                effort: AgentChatEffort::Medium,
                mode: AgentChatMode::Agent,
            },
        })
        .unwrap();
    let prompt = save_prompt(&ledger);
    let compatibility = compatibility();
    let (runtime, starts) = runtime(ledger.clone(), approved(&compatibility), compatibility);
    assert!(matches!(
        runtime.claim_prompt("daemon-a", HostEpoch(1)).unwrap(),
        gent_runtime::AgentChatPromptDispatchResult::Claimed(saved)
            if saved.message == prompt.message
    ));
    let request = PublicRunStartRequest {
        run_id: "run-a".into(),
        coordinator_id: "daemon-a".into(),
        host_epoch: HostEpoch(1),
        provider: DependencyProvider::Claude,
        executable: "ignored".into(),
        version: "ignored".into(),
        compatibility_entry: "ignored".into(),
    };
    assert_eq!(
        runtime.runs().start(request).unwrap().outcome,
        PublicRunOutcome::Started
    );
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    let session = runtime
        .record(
            "run-a".into(),
            "daemon-a",
            HostEpoch(1),
            PublicDriverFact::SessionEffect {
                event_id: "session-1".into(),
                effect: SessionEffect::SessionStarted {
                    provider_session_id: "private-session".into(),
                },
            },
        )
        .unwrap();
    assert_eq!(session, PublicDriverFactResult::Lifecycle(None));
    assert!(matches!(
        runtime
            .record(
                "run-a".into(),
                "daemon-a",
                HostEpoch(1),
                PublicDriverFact::SessionEffect {
                    event_id: "turn-1".into(),
                    effect: SessionEffect::Normalized {
                        event: NormalizedProviderEvent::TurnStarted {
                            turn_id: "turn-a".into()
                        },
                    },
                },
            )
            .unwrap(),
        PublicDriverFactResult::Lifecycle(Some(_))
    ));
    let activity = runtime
        .record(
            "run-a".into(),
            "daemon-a",
            HostEpoch(1),
            PublicDriverFact::Activity(gent_runtime::ProviderActivityFact {
                event_id: "activity-1".into(),
                activity: ConversationActivityFact::TurnStarted {
                    scope: ConversationActivityScope {
                        conversation_id: "conversation-a".into(),
                        run_id: "run-a".into(),
                        turn_id: "turn-a".into(),
                        host_epoch: HostEpoch(1),
                        cursor: 0,
                    },
                },
            }),
        )
        .unwrap();
    assert!(matches!(activity, PublicDriverFactResult::Activity(_)));
    assert_transcript(&runtime, prompt.message.turn_id);
    assert!(
        ledger
            .find_conversation_activity("conversation-a", "run-a")
            .unwrap()
            .is_some()
    );
}

fn save_prompt(ledger: &SqliteLedger) -> gent_types::AgentChatPromptSaved {
    ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-1".into()),
            receipt_id: ReceiptId("receipt-prompt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            disposition: AgentChatPromptDisposition::Send,
            text: "hello".into(),
        })
        .unwrap()
}

fn assert_transcript(
    runtime: &PublicDriversRuntime<SqliteLedger, Runner, Resolver>,
    turn_id: String,
) {
    let fact = PublicDriverFact::Transcript(gent_runtime::AgentChatTranscriptAppendRequest {
        conversation_id: AgentChatConversationId("conversation-a".into()),
        run_id: AgentChatRunId("run-a".into()),
        turn_id,
        event_id: "assistant-1".into(),
        kind: NormalizedTranscriptKind::AssistantMessage,
        text: "done".into(),
        is_partial: false,
    });
    assert!(matches!(
        runtime.record("run-a".into(), "daemon-a", HostEpoch(1), fact),
        Ok(PublicDriverFactResult::Transcript(_))
    ));
}

#[test]
fn observer_and_mismatched_profiles_cannot_construct_public_driver_authority() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let compatibility = compatibility();
    let observer = PublicDriversRuntime::new(
        ValidatedAuthorityProfile::Observer,
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger.clone(),
        compatibility.clone(),
        Runner(Arc::new(AtomicUsize::new(0))),
        Resolver,
    );
    assert_eq!(
        observer.unwrap_err(),
        PublicDriversRuntimeError::ObserverProfile
    );
    let approval = PublicDriverApproval {
        evidence_reference: "approved-evidence".into(),
        compatibility_manifest_sha256: "b".repeat(64),
    };
    let mismatch = PublicDriversRuntime::new(
        ValidatedAuthorityProfile::PreparedPublicDrivers(approval),
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger,
        compatibility,
        Runner(Arc::new(AtomicUsize::new(0))),
        Resolver,
    );
    assert_eq!(
        mismatch.unwrap_err(),
        PublicDriversRuntimeError::CompatibilityManifestMismatch
    );
}
