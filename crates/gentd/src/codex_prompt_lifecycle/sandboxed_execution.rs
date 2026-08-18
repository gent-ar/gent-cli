//! Sandbox-attested wrapper around the dormant Codex process execution edge.

use std::sync::Arc;

use gent_drivers::{codex_runner::CodexRunnerEffect, interrupt::ProcessTreeSignal};
use gent_ports::{
    PublicProviderRunError, PublicProviderRunner, SandboxedProviderPreflight,
    SandboxedProviderPreflightError,
};
use gent_types::{GoalProjection, RunVersionLock, SandboxLaunchProfile, SandboxedLaunchRequest};

use super::CodexPromptExecution;

/// Requires a fresh native sandbox attestation for every locked Codex start or resume.
///
/// This wrapper has no capability to enforce containment by itself. It denies delegation until
/// the injected daemon preflight proves a sandbox for the exact just-resolved executable lock.
pub(crate) struct SandboxedCodexPromptExecution<D, S> {
    inner: D,
    preflight: Arc<S>,
    profile: SandboxLaunchProfile,
}

impl<D, S> Clone for SandboxedCodexPromptExecution<D, S>
where
    D: Clone,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            preflight: Arc::clone(&self.preflight),
            profile: self.profile.clone(),
        }
    }
}

impl<D, S> SandboxedCodexPromptExecution<D, S> {
    /// Binds a private execution edge to a daemon-owned sandbox preparation port.
    #[must_use]
    pub(crate) fn new(inner: D, preflight: S, profile: SandboxLaunchProfile) -> Self {
        Self {
            inner,
            preflight: Arc::new(preflight),
            profile,
        }
    }
}

impl<D, S> PublicProviderRunner for SandboxedCodexPromptExecution<D, S>
where
    D: PublicProviderRunner,
    S: SandboxedProviderPreflight + 'static,
{
    fn start(&self, run_id: &str, lock: &RunVersionLock) -> Result<(), PublicProviderRunError> {
        self.attest(lock)?;
        self.inner.start(run_id, lock)
    }

    fn resume(
        &self,
        run_id: &str,
        lock: &RunVersionLock,
        session_id: &str,
    ) -> Result<(), PublicProviderRunError> {
        self.attest(lock)?;
        self.inner.resume(run_id, lock, session_id)
    }

    fn interrupt(&self, run_id: &str) -> Result<(), PublicProviderRunError> {
        self.inner.interrupt(run_id)
    }
}

impl<D, S> CodexPromptExecution for SandboxedCodexPromptExecution<D, S>
where
    D: CodexPromptExecution,
    S: SandboxedProviderPreflight + 'static,
{
    fn prepare_codex_prompt(
        &self,
        run_id: String,
        prompt: gent_drivers::codex_prompt_runner::CodexPromptStart,
    ) -> Result<(), PublicProviderRunError> {
        self.inner.prepare_codex_prompt(run_id, prompt)
    }

    fn cancel_codex_prompt(&self, run_id: &str) {
        self.inner.cancel_codex_prompt(run_id);
    }

    fn poll_codex_prompt(
        &self,
        run_id: &str,
    ) -> Result<Option<Vec<CodexRunnerEffect>>, PublicProviderRunError> {
        self.inner.poll_codex_prompt(run_id)
    }

    fn has_codex_session(&self, run_id: &str) -> bool {
        self.inner.has_codex_session(run_id)
    }

    fn submit_codex_prompt(
        &self,
        run_id: &str,
        prompt: &str,
        goal: Option<&GoalProjection>,
    ) -> Result<(), PublicProviderRunError> {
        self.inner.submit_codex_prompt(run_id, prompt, goal)
    }

    fn signal_codex_process(
        &self,
        run_id: &str,
        signal: ProcessTreeSignal,
    ) -> Result<(), PublicProviderRunError> {
        self.inner.signal_codex_process(run_id, signal)
    }
}

impl<D, S> SandboxedCodexPromptExecution<D, S>
where
    S: SandboxedProviderPreflight,
{
    fn attest(&self, lock: &RunVersionLock) -> Result<(), PublicProviderRunError> {
        let request = SandboxedLaunchRequest {
            lock: lock.clone(),
            profile: self.profile.clone(),
        };
        let attestation = self.preflight.preflight(&request).map_err(map_preflight)?;
        (attestation.profile_digest_sha256 == self.profile.digest_sha256()
            && attestation.executable_digest_sha256 == lock.digest_sha256
            && attestation.executable_file_identity == lock.file_identity)
            .then_some(())
            .ok_or_else(|| PublicProviderRunError::Failed("sandbox preflight rejected".into()))
    }
}

fn map_preflight(error: SandboxedProviderPreflightError) -> PublicProviderRunError {
    match error {
        SandboxedProviderPreflightError::LockChanged => PublicProviderRunError::ProviderChanged,
        SandboxedProviderPreflightError::Unavailable
        | SandboxedProviderPreflightError::ProfileRejected
        | SandboxedProviderPreflightError::AttestationRejected => {
            PublicProviderRunError::Failed("sandbox preflight unavailable".into())
        }
    }
}

#[cfg(test)]
#[path = "sandboxed_execution_tests.rs"]
mod tests;
