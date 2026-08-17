use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use ed25519_dalek::{Signer, SigningKey};
use gent_adapters::compatibility::{
    CompatibilityEntry, CompatibilityManifest, SignedCompatibilityManifest, TrustedKeySet,
};
use gent_adapters::compatibility_cache::CachedCompatibilityManifest;
use gent_drivers::codex_prompt_runner::CodexPromptStart;
use gent_drivers::codex_runner::CodexRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{
    AgentChatLedger, AgentChatPromptDispatchLedger, AgentChatPromptLedger, PublicProviderResolver,
    PublicProviderRunError, PublicProviderRunner, TranscriptLedger,
};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, CapabilitySet, HostEpoch, NormalizedLifecycleSignal,
    NormalizedProviderEvent, ReceiptId, RunVersionLock, TurnPhase,
};

use crate::authority_profile::{AuthorityProfileConfig, PublicDriverApproval, PublicDriverRequest};
use crate::codex_prompt_lifecycle::{
    CodexPromptDispatchOutcome, CodexPromptExecution, CodexPromptLifecycle,
};
use crate::compatibility_assessment::CompatibilityAssessment;
use crate::public_driver_runtime::PublicDriversRuntime;

#[derive(Clone, Debug, Default)]
pub(crate) struct Runner {
    pub(crate) state: Arc<Mutex<State>>,
}

#[derive(Default, Debug)]
pub(crate) struct State {
    pending: Option<(String, CodexPromptStart)>,
    starts: usize,
    effects: VecDeque<Vec<CodexRunnerEffect>>,
    pub(crate) poll_failure: bool,
    session_active: bool,
    submitted: Vec<String>,
}

impl PublicProviderRunner for Runner {
    fn start(&self, run_id: &str, lock: &RunVersionLock) -> Result<(), PublicProviderRunError> {
        let mut state = self.state.lock().unwrap();
        assert_eq!(lock.provider, "codex");
        assert_eq!(
            state.pending.as_ref().map(|entry| entry.0.as_str()),
            Some(run_id)
        );
        state.starts += 1;
        state.session_active = true;
        Ok(())
    }

    fn resume(&self, _: &str, _: &RunVersionLock, _: &str) -> Result<(), PublicProviderRunError> {
        Err(PublicProviderRunError::Failed("unused resume".into()))
    }

    fn interrupt(&self, _: &str) -> Result<(), PublicProviderRunError> {
        Ok(())
    }
}

impl CodexPromptExecution for Runner {
    fn prepare_codex_prompt(
        &self,
        run_id: String,
        prompt: CodexPromptStart,
    ) -> Result<(), PublicProviderRunError> {
        let mut state = self.state.lock().unwrap();
        if state.pending.is_some() {
            return Err(PublicProviderRunError::Failed("duplicate prompt".into()));
        }
        state.pending = Some((run_id, prompt));
        Ok(())
    }
    fn cancel_codex_prompt(&self, _: &str) {
        self.state.lock().unwrap().pending = None;
    }
    fn poll_codex_prompt(
        &self,
        _: &str,
    ) -> Result<Option<Vec<CodexRunnerEffect>>, PublicProviderRunError> {
        let mut state = self.state.lock().unwrap();
        if state.poll_failure {
            return Err(PublicProviderRunError::Failed(
                "private runner detail".into(),
            ));
        }
        Ok(state.effects.pop_front())
    }

    fn has_codex_session(&self, _: &str) -> bool {
        self.state.lock().unwrap().session_active
    }

    fn submit_codex_prompt(&self, _: &str, prompt: &str) -> Result<(), PublicProviderRunError> {
        self.state.lock().unwrap().submitted.push(prompt.into());
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct Resolver;

impl PublicProviderResolver for Resolver {
    fn resolve(&self, provider: &str) -> Result<RunVersionLock, PublicProviderRunError> {
        (provider == "codex")
            .then(lock)
            .ok_or(PublicProviderRunError::CompatibilityDenied)
    }
}

fn lock() -> RunVersionLock {
    RunVersionLock {
        provider: "codex".into(),
        canonical_path: "/verified/codex".into(),
        file_identity: "1:2".into(),
        digest_sha256: "a".repeat(64),
        version: "0.144.1".into(),
        compatibility_entry: "codex-0.144.1".into(),
    }
}

pub(crate) fn compatibility() -> CompatibilityAssessment {
    let key = SigningKey::from_bytes(&[7; 32]);
    let payload = CompatibilityManifest {
        manifest_version: 1,
        expires_at_unix_seconds: 20,
        entries: vec![CompatibilityEntry {
            id: "codex-0.144.1".into(),
            provider: "codex".into(),
            version: "0.144.1".into(),
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
    CompatibilityAssessment::configured(
        keys.clone(),
        CachedCompatibilityManifest::verify(manifest, &keys, 1).unwrap(),
        10,
    )
}

pub(crate) fn profile(
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

#[test]
fn codex_host_reserves_then_persists_normalized_facts_and_settles() {
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
                provider: AgentChatProvider::Codex,
                model: "gpt-5.6".into(),
                effort: AgentChatEffort::Medium,
                mode: AgentChatMode::Agent,
            },
        })
        .unwrap();
    let prompt = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-a".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            disposition: AgentChatPromptDisposition::Send,
            text: "hello".into(),
        })
        .unwrap();
    let runner = Runner::default();
    runner.state.lock().unwrap().effects.push_back(vec![
        CodexRunnerEffect::Fact(PublicWireFact::SessionStarted {
            provider_session_id: "private-thread".into(),
        }),
        CodexRunnerEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::Output {
            text: "hello back".into(),
        })),
        CodexRunnerEffect::Fact(PublicWireFact::Lifecycle(
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
    let mut host = CodexPromptLifecycle::new(runtime, "daemon-a".into(), Some("/work".into()));
    assert_eq!(
        host.dispatch_next(HostEpoch(1)).unwrap(),
        CodexPromptDispatchOutcome::Started {
            run_id: "run-a".into()
        }
    );
    assert_eq!(runner.state.lock().unwrap().starts, 1);
    assert!(!host.poll("run-a", HostEpoch(1)).unwrap().unwrap().exited);
    let transcript = ledger
        .normalized_transcript_page(&conversation_id, 0, 10)
        .unwrap();
    assert_eq!(transcript.events.len(), 1);
    assert_eq!(transcript.events[0].text, "hello back");
    ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-b".into()),
            receipt_id: ReceiptId("prompt-receipt-b".into()),
            host_epoch: HostEpoch(1),
            conversation_id,
            disposition: AgentChatPromptDisposition::Send,
            text: "follow up".into(),
        })
        .unwrap();
    assert!(matches!(
        host.dispatch_next(HostEpoch(1)).unwrap(),
        CodexPromptDispatchOutcome::Started { .. }
    ));
    let state = runner.state.lock().unwrap();
    assert_eq!(state.starts, 1);
    assert_eq!(state.submitted, ["follow up"]);
    drop(state);
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
            .unwrap()
            .is_none()
    );
    assert_eq!(prompt.message.text, "hello");
}
