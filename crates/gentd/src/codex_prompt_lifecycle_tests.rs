use crate::authority_profile::{AuthorityProfileConfig, PublicDriverApproval, PublicDriverRequest};
use crate::codex_prompt_lifecycle::CodexPromptExecution;
use crate::compatibility_assessment::CompatibilityAssessment;
use ed25519_dalek::{Signer, SigningKey};
use gent_adapters::compatibility::{
    CompatibilityEntry, CompatibilityManifest, SignedCompatibilityManifest, TrustedKeySet,
};
use gent_adapters::compatibility_cache::CachedCompatibilityManifest;
use gent_drivers::codex_prompt_runner::CodexPromptStart;
use gent_drivers::codex_runner::CodexRunnerEffect;
use gent_drivers::codex_session::CodexTurnOptions;
use gent_drivers::interrupt::ProcessTreeSignal;
use gent_ports::{PublicProviderResolver, PublicProviderRunError, PublicProviderRunner};
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, RunVersionLock,
};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
#[derive(Clone, Debug, Default)]
pub(crate) struct Runner {
    pub(crate) state: Arc<Mutex<State>>,
}
#[derive(Default, Debug)]
pub(crate) struct State {
    pending: Option<(String, CodexPromptStart)>,
    pub(crate) starts: usize,
    pub(crate) effects: VecDeque<Vec<CodexRunnerEffect>>,
    pub(crate) poll_failure: bool,
    session_active: bool,
    pub(crate) submitted: Vec<String>,
    pub(crate) prepared_goals: Vec<Option<gent_types::GoalProjection>>,
    pub(crate) submitted_goals: Vec<Option<gent_types::GoalProjection>>,
    pub(crate) resumes: usize,
    pub(crate) signals: Vec<ProcessTreeSignal>,
    pub(crate) turn_interrupts: Vec<String>,
    pub(crate) releases: Vec<String>,
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

    fn resume(
        &self,
        run_id: &str,
        _: &RunVersionLock,
        _: &str,
    ) -> Result<(), PublicProviderRunError> {
        let mut state = self.state.lock().unwrap();
        assert_eq!(
            state.pending.as_ref().map(|entry| entry.0.as_str()),
            Some(run_id)
        );
        state.resumes += 1;
        state.session_active = true;
        Ok(())
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
        state.prepared_goals.push(prompt.goal.clone());
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
        let effects = state.effects.pop_front();
        if effects.as_ref().is_some_and(|effects| {
            effects
                .iter()
                .any(|effect| matches!(effect, CodexRunnerEffect::Exited { .. }))
        }) {
            state.session_active = false;
            state.pending = None;
        }
        Ok(effects)
    }
    fn has_codex_session(&self, _: &str) -> bool {
        self.state.lock().unwrap().session_active
    }

    fn release_codex_session(&self, run_id: &str) -> Result<(), PublicProviderRunError> {
        let mut state = self.state.lock().unwrap();
        state.session_active = false;
        state.releases.push(run_id.into());
        Ok(())
    }

    fn refresh_codex_mcp_config(&self, _: &str) -> Result<bool, PublicProviderRunError> {
        Ok(false)
    }

    fn submit_codex_prompt(
        &self,
        _: &str,
        prompt: &str,
        goal: Option<&gent_types::GoalProjection>,
        _: &[serde_json::Value],
    ) -> Result<(), PublicProviderRunError> {
        let mut state = self.state.lock().unwrap();
        state.submitted.push(prompt.into());
        state.submitted_goals.push(goal.cloned());
        Ok(())
    }

    fn signal_codex_process(
        &self,
        _: &str,
        signal: ProcessTreeSignal,
    ) -> Result<(), PublicProviderRunError> {
        self.state.lock().unwrap().signals.push(signal);
        Ok(())
    }

    fn interrupt_codex_turn(&self, run_id: &str) -> Result<(), PublicProviderRunError> {
        self.state
            .lock()
            .unwrap()
            .turn_interrupts
            .push(run_id.into());
        Ok(())
    }

    fn respond_codex_control(
        &self,
        _: &str,
        _: &str,
        _: gent_drivers::codex_control::CodexControlDecision,
        _: Option<serde_json::Value>,
    ) -> Result<(), PublicProviderRunError> {
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
pub(crate) fn lock() -> RunVersionLock {
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

pub(crate) fn selection() -> AgentChatSelection {
    AgentChatSelection {
        provider: AgentChatProvider::Codex,
        model: "gpt-5.6".into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Agent,
    }
}

pub(crate) fn assert_prepared_options(runner: &Runner) {
    let expected = CodexTurnOptions::from_selection_with_permissions(
        &selection(),
        Some("/workspace-a"),
        gent_types::PermissionMode::Default,
    )
    .unwrap();
    let state = runner.state.lock().unwrap();
    assert_eq!(
        state.pending.as_ref().map(|entry| &entry.1.turn_options),
        Some(&expected)
    );
    assert_eq!(
        state.pending.as_ref().map(|entry| {
            entry
                .1
                .selected_mcp_source_names
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        }),
        Some(Vec::new())
    );
}
