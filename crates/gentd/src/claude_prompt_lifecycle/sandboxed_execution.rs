//! Sandbox-attested wrapper around a dormant Claude process execution edge.

use std::sync::Arc;

use gent_drivers::{claude_runner::ClaudeRunnerEffect, interrupt::ProcessTreeSignal};
use gent_ports::{
    PublicProviderRunError, PublicProviderRunner, SandboxedProviderPreflight,
    SandboxedProviderPreflightError,
};
use gent_types::{RunVersionLock, SandboxLaunchProfile, SandboxedLaunchRequest};

use super::{ClaudePromptExecution, ClaudePromptStart};

/// Requires a fresh native-sandbox attestation before each Claude process start or resume.
///
/// It owns no process and cannot make containment enforceable itself. It only prevents the
/// underlying execution edge from receiving a start until the injected daemon preflight proves
/// containment for this exact executable lock and profile.
#[derive(Debug)]
pub(crate) struct SandboxedClaudePromptExecution<D, S> {
    inner: D,
    preflight: Arc<S>,
    expected_lock: RunVersionLock,
    profile: SandboxLaunchProfile,
}

impl<D: Clone, S> Clone for SandboxedClaudePromptExecution<D, S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            preflight: Arc::clone(&self.preflight),
            expected_lock: self.expected_lock.clone(),
            profile: self.profile.clone(),
        }
    }
}

impl<D, S> SandboxedClaudePromptExecution<D, S> {
    /// Binds an execution edge to one daemon-locked executable and sandbox profile.
    #[must_use]
    pub(crate) fn new(inner: D, preflight: S, request: SandboxedLaunchRequest) -> Self {
        Self {
            inner,
            preflight: Arc::new(preflight),
            expected_lock: request.lock,
            profile: request.profile,
        }
    }
}

impl<D, S> PublicProviderRunner for SandboxedClaudePromptExecution<D, S>
where
    D: PublicProviderRunner,
    S: SandboxedProviderPreflight + std::fmt::Debug + 'static,
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

impl<D, S> ClaudePromptExecution for SandboxedClaudePromptExecution<D, S>
where
    D: ClaudePromptExecution,
    S: SandboxedProviderPreflight + std::fmt::Debug + 'static,
{
    fn prepare_claude_prompt(
        &self,
        run_id: String,
        prompt: ClaudePromptStart,
    ) -> Result<(), PublicProviderRunError> {
        self.inner.prepare_claude_prompt(run_id, prompt)
    }

    fn cancel_claude_prompt(&self, run_id: &str) {
        self.inner.cancel_claude_prompt(run_id);
    }

    fn poll_claude_prompt(
        &self,
        run_id: &str,
    ) -> Result<Option<Vec<ClaudeRunnerEffect>>, PublicProviderRunError> {
        self.inner.poll_claude_prompt(run_id)
    }

    fn signal_claude_process(
        &self,
        run_id: &str,
        signal: ProcessTreeSignal,
    ) -> Result<(), PublicProviderRunError> {
        self.inner.signal_claude_process(run_id, signal)
    }
}

impl<D, S> SandboxedClaudePromptExecution<D, S>
where
    S: SandboxedProviderPreflight,
{
    fn attest(&self, lock: &RunVersionLock) -> Result<(), PublicProviderRunError> {
        if lock != &self.expected_lock {
            return Err(PublicProviderRunError::ProviderChanged);
        }
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
