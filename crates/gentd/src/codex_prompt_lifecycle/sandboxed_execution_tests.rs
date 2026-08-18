use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use gent_ports::{
    PublicProviderRunError, PublicProviderRunner, SandboxedProviderPreflight,
    SandboxedProviderPreflightError,
};
use gent_types::{
    RunVersionLock, SandboxBackendId, SandboxEnforcement, SandboxLaunchAttestation,
    SandboxLaunchProfile, SandboxNetworkPolicy, SandboxResourceLimits, SandboxedLaunchRequest,
};

use super::SandboxedCodexPromptExecution;

#[derive(Clone, Default)]
struct Inner(Arc<Mutex<Vec<String>>>);

impl PublicProviderRunner for Inner {
    fn start(&self, run_id: &str, _: &RunVersionLock) -> Result<(), PublicProviderRunError> {
        self.0.lock().unwrap().push(format!("start:{run_id}"));
        Ok(())
    }

    fn resume(
        &self,
        run_id: &str,
        _: &RunVersionLock,
        _: &str,
    ) -> Result<(), PublicProviderRunError> {
        self.0.lock().unwrap().push(format!("resume:{run_id}"));
        Ok(())
    }

    fn interrupt(&self, _: &str) -> Result<(), PublicProviderRunError> {
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum ResultKind {
    Unavailable,
    ChangedLock,
    Attested,
    WrongAttestation,
}

struct Preflight(ResultKind);

impl SandboxedProviderPreflight for Preflight {
    fn preflight(
        &self,
        request: &SandboxedLaunchRequest,
    ) -> Result<SandboxLaunchAttestation, SandboxedProviderPreflightError> {
        match self.0 {
            ResultKind::Unavailable => Err(SandboxedProviderPreflightError::Unavailable),
            ResultKind::ChangedLock => Err(SandboxedProviderPreflightError::LockChanged),
            ResultKind::Attested => request
                .attest_after_lock_recheck(
                    &request.lock,
                    SandboxBackendId::new("test-native-sandbox".into()).unwrap(),
                    SandboxEnforcement::Enforced,
                )
                .map_err(|_| SandboxedProviderPreflightError::AttestationRejected),
            ResultKind::WrongAttestation => Ok(SandboxLaunchAttestation {
                backend: SandboxBackendId::new("test-native-sandbox".into()).unwrap(),
                profile_digest_sha256: "b".repeat(64),
                executable_digest_sha256: request.lock.digest_sha256.clone(),
                executable_file_identity: request.lock.file_identity.clone(),
            }),
        }
    }
}

fn profile() -> SandboxLaunchProfile {
    SandboxLaunchProfile::new(
        &PathBuf::from("/workspace"),
        &[PathBuf::from("/workspace")],
        &[],
        vec!["TERM".into()],
        SandboxNetworkPolicy::Disabled,
        SandboxResourceLimits {
            max_processes: 4,
            max_memory_bytes: 1_000_000,
            max_cpu_time_ms: 60_000,
        },
    )
    .unwrap()
}

fn lock() -> RunVersionLock {
    RunVersionLock {
        provider: "codex".into(),
        canonical_path: "/private/gent/codex".into(),
        file_identity: "1:2".into(),
        digest_sha256: "a".repeat(64),
        version: "0.147.0".into(),
        compatibility_entry: "codex-0.147.0".into(),
    }
}

fn request() -> SandboxedLaunchRequest {
    SandboxedLaunchRequest {
        lock: lock(),
        profile: profile(),
    }
}

#[test]
fn unavailable_or_changed_preflight_never_delegates_to_the_process_runner() {
    for preflight in [ResultKind::Unavailable, ResultKind::ChangedLock] {
        let inner = Inner::default();
        let execution =
            SandboxedCodexPromptExecution::new(inner.clone(), Preflight(preflight), request());
        let error = execution.start("run-a", &lock()).unwrap_err();
        match preflight {
            ResultKind::Unavailable => assert!(matches!(
                error,
                PublicProviderRunError::Failed(message) if message == "sandbox preflight unavailable"
            )),
            ResultKind::ChangedLock => assert_eq!(error, PublicProviderRunError::ProviderChanged),
            _ => unreachable!(),
        }
        assert!(inner.0.lock().unwrap().is_empty());
    }
}

#[test]
fn mismatched_attestation_never_delegates_to_the_process_runner() {
    let inner = Inner::default();
    let execution = SandboxedCodexPromptExecution::new(
        inner.clone(),
        Preflight(ResultKind::WrongAttestation),
        request(),
    );
    assert!(matches!(
        execution.start("run-a", &lock()),
        Err(PublicProviderRunError::Failed(message)) if message == "sandbox preflight rejected"
    ));
    assert!(inner.0.lock().unwrap().is_empty());
}

#[test]
fn attested_preflight_is_rechecked_before_every_start_and_resume() {
    let inner = Inner::default();
    let execution = SandboxedCodexPromptExecution::new(
        inner.clone(),
        Preflight(ResultKind::Attested),
        request(),
    );
    execution.start("run-a", &lock()).unwrap();
    execution.resume("run-b", &lock(), "thread-a").unwrap();
    assert_eq!(
        inner.0.lock().unwrap().as_slice(),
        ["start:run-a", "resume:run-b"]
    );
    let mut changed = lock();
    changed.digest_sha256 = "b".repeat(64);
    assert_eq!(
        execution.start("run-c", &changed),
        Err(PublicProviderRunError::ProviderChanged)
    );
    assert_eq!(
        inner.0.lock().unwrap().as_slice(),
        ["start:run-a", "resume:run-b"]
    );
}
