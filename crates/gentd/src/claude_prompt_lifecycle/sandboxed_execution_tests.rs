use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gent_drivers::{claude_runner::ClaudeRunnerEffect, interrupt::ProcessTreeSignal};
use gent_ports::{
    PublicProviderRunError, PublicProviderRunner, SandboxedProviderPreflight,
    SandboxedProviderPreflightError,
};
use gent_types::{
    RunVersionLock, SandboxBackendId, SandboxLaunchAttestation, SandboxLaunchProfile,
    SandboxNetworkPolicy, SandboxResourceLimits, SandboxedLaunchRequest,
};

use super::super::{ClaudePromptExecution, ClaudePromptStart};
use super::SandboxedClaudePromptExecution;

#[derive(Clone, Debug, Default)]
struct Execution {
    starts: Arc<AtomicUsize>,
}

impl PublicProviderRunner for Execution {
    fn start(&self, _: &str, _: &RunVersionLock) -> Result<(), PublicProviderRunError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn resume(&self, _: &str, _: &RunVersionLock, _: &str) -> Result<(), PublicProviderRunError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn interrupt(&self, _: &str) -> Result<(), PublicProviderRunError> {
        Ok(())
    }
}

impl ClaudePromptExecution for Execution {
    fn prepare_claude_prompt(
        &self,
        _: String,
        _: ClaudePromptStart,
    ) -> Result<(), PublicProviderRunError> {
        Ok(())
    }

    fn cancel_claude_prompt(&self, _: &str) {}

    fn poll_claude_prompt(
        &self,
        _: &str,
    ) -> Result<Option<Vec<ClaudeRunnerEffect>>, PublicProviderRunError> {
        Ok(None)
    }

    fn signal_claude_process(
        &self,
        _: &str,
        _: ProcessTreeSignal,
    ) -> Result<(), PublicProviderRunError> {
        Ok(())
    }
}

#[derive(Clone, Debug)]
enum ResultKind {
    Unavailable,
    Attest,
    WrongAttestation,
}

#[derive(Clone, Debug)]
struct Preflight(ResultKind);

impl SandboxedProviderPreflight for Preflight {
    fn preflight(
        &self,
        request: &SandboxedLaunchRequest,
    ) -> Result<SandboxLaunchAttestation, SandboxedProviderPreflightError> {
        if matches!(self.0, ResultKind::Unavailable) {
            return Err(SandboxedProviderPreflightError::Unavailable);
        }
        Ok(SandboxLaunchAttestation {
            backend: SandboxBackendId::new("test-backend".into()).unwrap(),
            profile_digest_sha256: if matches!(self.0, ResultKind::WrongAttestation) {
                "0".repeat(64)
            } else {
                request.profile.digest_sha256()
            },
            executable_digest_sha256: request.lock.digest_sha256.clone(),
            executable_file_identity: request.lock.file_identity.clone(),
        })
    }
}

fn lock() -> RunVersionLock {
    RunVersionLock {
        provider: "claude".into(),
        canonical_path: "/workspace/bin/claude".into(),
        file_identity: "1:2".into(),
        digest_sha256: "a".repeat(64),
        version: "1.0.0".into(),
        compatibility_entry: "claude-test".into(),
    }
}

fn profile() -> SandboxLaunchProfile {
    let workspace = PathBuf::from("/workspace");
    SandboxLaunchProfile::new(
        &workspace,
        std::slice::from_ref(&workspace),
        std::slice::from_ref(&workspace),
        vec!["LANG".into()],
        SandboxNetworkPolicy::Disabled,
        SandboxResourceLimits {
            max_processes: 4,
            max_memory_bytes: 1,
            max_cpu_time_ms: 1,
        },
    )
    .unwrap()
}

#[test]
fn unavailable_sandbox_fails_before_the_inner_execution_receives_a_start() {
    let execution = Execution::default();
    let guarded = SandboxedClaudePromptExecution::new(
        execution.clone(),
        Preflight(ResultKind::Unavailable),
        profile(),
    );
    assert_eq!(
        guarded.start("run-1", &lock()),
        Err(PublicProviderRunError::Failed(
            "sandbox preflight unavailable".into()
        ))
    );
    assert_eq!(execution.starts.load(Ordering::SeqCst), 0);
}

#[test]
fn mismatched_attestation_fails_before_the_inner_execution_receives_a_resume() {
    let execution = Execution::default();
    let guarded = SandboxedClaudePromptExecution::new(
        execution.clone(),
        Preflight(ResultKind::WrongAttestation),
        profile(),
    );
    assert_eq!(
        guarded.resume("run-1", &lock(), "session-1"),
        Err(PublicProviderRunError::Failed(
            "sandbox preflight rejected".into()
        ))
    );
    assert_eq!(execution.starts.load(Ordering::SeqCst), 0);
}

#[test]
fn exact_attestation_is_required_before_start_or_resume_can_reach_execution() {
    let execution = Execution::default();
    let guarded = SandboxedClaudePromptExecution::new(
        execution.clone(),
        Preflight(ResultKind::Attest),
        profile(),
    );
    guarded.start("run-1", &lock()).unwrap();
    guarded.resume("run-2", &lock(), "session-1").unwrap();
    assert_eq!(execution.starts.load(Ordering::SeqCst), 2);
}
