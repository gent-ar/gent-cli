use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use ed25519_dalek::{Signer, SigningKey};
use gent_adapters::compatibility::{
    CompatibilityEntry, CompatibilityManifest, SignedCompatibilityManifest, TrustedKeySet,
};
use gent_adapters::compatibility_cache::CachedCompatibilityManifest;
use gent_drivers::claude_runner::ClaudeRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{
    AgentChatLedger, AgentChatPromptLedger, PublicProviderResolver, PublicProviderRunError,
    PublicProviderRunner,
};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, CapabilitySet, HostEpoch, NormalizedLifecycleSignal,
    NormalizedProviderEvent, ReceiptId, RunVersionLock, TurnPhase,
};

use crate::approved_claude_host::ApprovedClaudeHost;
use crate::authority_profile::{AuthorityProfileConfig, PublicDriverApproval, PublicDriverRequest};
use crate::claude_prompt_lifecycle::{ClaudePromptExecution, ClaudePromptStart};
use crate::compatibility_assessment::CompatibilityAssessment;
use crate::public_driver_runtime::PublicDriversRuntime;

#[derive(Clone, Default, Debug)]
struct Runner(Arc<Mutex<State>>);
#[derive(Default, Debug)]
struct State {
    pending: Option<String>,
    starts: usize,
    resumes: usize,
    effects: VecDeque<Vec<ClaudeRunnerEffect>>,
}

impl PublicProviderRunner for Runner {
    fn start(&self, _: &str, lock: &RunVersionLock) -> Result<(), PublicProviderRunError> {
        assert_eq!(lock.provider, "claude");
        let mut state = self.0.lock().unwrap();
        assert!(state.pending.take().is_some());
        state.starts += 1;
        Ok(())
    }

    fn resume(&self, _: &str, _: &RunVersionLock, _: &str) -> Result<(), PublicProviderRunError> {
        let mut state = self.0.lock().unwrap();
        assert!(state.pending.take().is_some());
        state.resumes += 1;
        Ok(())
    }

    fn interrupt(&self, _: &str) -> Result<(), PublicProviderRunError> {
        Ok(())
    }
}

impl ClaudePromptExecution for Runner {
    fn prepare_claude_prompt(
        &self,
        run_id: String,
        _: ClaudePromptStart,
    ) -> Result<(), PublicProviderRunError> {
        self.0.lock().unwrap().pending = Some(run_id);
        Ok(())
    }
    fn cancel_claude_prompt(&self, _: &str) {
        self.0.lock().unwrap().pending = None;
    }
    fn poll_claude_prompt(
        &self,
        _: &str,
    ) -> Result<Option<Vec<ClaudeRunnerEffect>>, PublicProviderRunError> {
        Ok(self.0.lock().unwrap().effects.pop_front())
    }
}

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
        digest_sha256: "b".repeat(64),
        version: "2.1.0".into(),
        compatibility_entry: "claude-2.1.0".into(),
    }
}

fn compatibility() -> CompatibilityAssessment {
    let key = SigningKey::from_bytes(&[8; 32]);
    let payload = CompatibilityManifest {
        manifest_version: 1,
        expires_at_unix_seconds: 20,
        entries: vec![CompatibilityEntry {
            id: "claude-2.1.0".into(),
            provider: "claude".into(),
            version: "2.1.0".into(),
            digest_sha256: "b".repeat(64),
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
    CompatibilityAssessment::configured(
        keys.clone(),
        CachedCompatibilityManifest::verify(manifest, &keys, 1).unwrap(),
        10,
    )
}

fn profile(
    compatibility: &CompatibilityAssessment,
) -> crate::authority_profile::ValidatedAuthorityProfile {
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

fn prompt(ledger: &SqliteLedger, conversation_id: &AgentChatConversationId, key: &str) {
    ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(format!("request-{key}")),
            receipt_id: ReceiptId(format!("receipt-{key}")),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            disposition: AgentChatPromptDisposition::Send,
            text: format!("message-{key}"),
        })
        .unwrap();
}

#[test]
fn ready_settles_but_keeps_one_shot_claude_binding_until_exit_then_resumes() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = AgentChatConversationId("conversation-a".into());
    ledger
        .create_agent_chat_conversation(&AgentChatConversationCreate {
            receipt_id: ReceiptId("conversation-receipt".into()),
            idempotency_key: "conversation-key".into(),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            run_id: AgentChatRunId("run-a".into()),
            selection: AgentChatSelection {
                provider: AgentChatProvider::Claude,
                model: "claude-test".into(),
                effort: AgentChatEffort::Medium,
                mode: AgentChatMode::Agent,
            },
        })
        .unwrap();
    prompt(&ledger, &conversation_id, "a");
    let runner = Runner::default();
    runner.0.lock().unwrap().effects.push_back(vec![
        ClaudeRunnerEffect::Fact(PublicWireFact::SessionStarted {
            provider_session_id: "private-session".into(),
        }),
        ClaudeRunnerEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::Output {
            text: "done".into(),
        })),
        ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::RootPhase {
                phase: TurnPhase::Ready,
            },
        )),
    ]);
    let compatibility = compatibility();
    let runtime = PublicDriversRuntime::new(
        profile(&compatibility),
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger.clone(),
        compatibility,
        runner.clone(),
        Resolver,
    )
    .unwrap();
    let mut host = ApprovedClaudeHost::new(runtime, "daemon-a".into(), HostEpoch(1), 1);
    let resumed = host.tick().unwrap();
    assert!(matches!(
        resumed.dispatch,
        Some(crate::claude_prompt_lifecycle::ClaudePromptDispatchOutcome::Started { .. })
    ));
    let ready = host.tick().unwrap();
    assert_eq!(ready.batch.facts, 3);
    prompt(&ledger, &conversation_id, "b");
    assert_eq!(
        host.tick().unwrap().dispatch,
        None,
        "ready process must remain bound until exit"
    );
    runner
        .0
        .lock()
        .unwrap()
        .effects
        .push_back(vec![ClaudeRunnerEffect::Exited { code: Some(0) }]);
    let resumed = host.tick().unwrap();
    assert!(matches!(
        resumed.dispatch,
        Some(crate::claude_prompt_lifecycle::ClaudePromptDispatchOutcome::Started { .. })
    ));
    let state = runner.0.lock().unwrap();
    assert_eq!((state.starts, state.resumes), (1, 1));
}
