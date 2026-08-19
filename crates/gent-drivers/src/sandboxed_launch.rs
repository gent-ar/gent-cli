//! Atomic containment-and-spawn boundary for public provider processes.
//!
//! A sandbox attestation alone is not authorization to use a separate launcher: it can become
//! stale between verification and `Command::spawn`. This port owns both operations in one
//! platform implementation. `SystemLauncher` deliberately does not implement this trait.

use std::sync::Arc;

use gent_types::SandboxedLaunchRequest;

use crate::lock::LockError;
use crate::supervisor::{ProcessLauncher, ProviderLaunch, ProviderProcess, SupervisorError};

/// Failure from the platform-owned containment and spawn operation.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SandboxedProviderLaunchError {
    #[error("provider executable changed before contained launch")]
    LockChanged,
    #[error("a native sandbox backend is unavailable")]
    Unavailable,
    #[error("sandbox profile was rejected")]
    ProfileRejected,
    #[error("sandbox backend could not prove containment")]
    ContainmentRejected,
    #[error("contained provider launch failed: {0}")]
    LaunchFailed(String),
}

/// The only process-spawning port accepted by a sandbox-required authority.
///
/// Implementations must recheck `request.lock`, apply `request.profile`, and start the exact
/// `launch` as one trusted platform operation. Returning a process means the launched process is
/// already contained; implementations must never return an unenforced fallback process.
pub trait SandboxedProviderLaunch: Send + Sync {
    /// The process tree owned after a successful contained spawn.
    type Process: ProviderProcess + 'static;

    /// Rechecks containment inputs and starts one contained provider process atomically.
    ///
    /// # Errors
    /// Returns without a provider process when containment or the immutable executable fence
    /// cannot be proved.
    fn launch_sandboxed(
        &self,
        request: &SandboxedLaunchRequest,
        launch: &ProviderLaunch,
    ) -> Result<Self::Process, SandboxedProviderLaunchError>;
}

/// A `ProcessLauncher` adapter that keeps one immutable sandbox request bound to every spawn.
///
/// It is the only launcher that dormant sandbox-required compositions may construct. The inner
/// port receives the profile and spawn request together, so it cannot attest and then delegate to
/// an unrelated `SystemLauncher`.
#[derive(Clone, Debug)]
pub struct SandboxedLauncher<S> {
    request: SandboxedLaunchRequest,
    inner: Arc<S>,
}

impl<S> SandboxedLauncher<S> {
    /// Binds a platform containment-and-spawn port to a single immutable executable/profile.
    #[must_use]
    pub fn new(request: SandboxedLaunchRequest, inner: S) -> Self {
        Self {
            request,
            inner: Arc::new(inner),
        }
    }
}

impl<S> ProcessLauncher for SandboxedLauncher<S>
where
    S: SandboxedProviderLaunch + 'static,
{
    type Process = S::Process;

    fn launch(&self, launch: &ProviderLaunch) -> Result<Self::Process, SupervisorError> {
        let expected = &self.request.lock;
        (launch.provider == expected.provider
            && launch.executable.to_string_lossy() == expected.canonical_path)
            .then_some(())
            .ok_or(SupervisorError::Lock(LockError::ProviderChanged))?;
        self.inner
            .launch_sandboxed(&self.request, launch)
            .map_err(map_error)
    }
}

fn map_error(error: SandboxedProviderLaunchError) -> SupervisorError {
    match error {
        SandboxedProviderLaunchError::LockChanged => {
            SupervisorError::Lock(LockError::ProviderChanged)
        }
        other => SupervisorError::Launch(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use gent_types::{
        RunVersionLock, SandboxLaunchProfile, SandboxNetworkPolicy, SandboxResourceLimits,
    };

    use super::*;
    use crate::interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal};
    use crate::supervisor::LaunchIntent;

    #[derive(Debug)]
    struct Process;
    impl ProcessTreeControl for Process {
        fn signal_tree(&self, _: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
            Ok(())
        }
    }
    impl ProviderProcess for Process {
        fn write_frame(&self, _: &[u8]) -> Result<(), ProcessTreeError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct LaunchPort(Mutex<Vec<(SandboxedLaunchRequest, ProviderLaunch)>>);
    impl SandboxedProviderLaunch for LaunchPort {
        type Process = Process;
        fn launch_sandboxed(
            &self,
            request: &SandboxedLaunchRequest,
            launch: &ProviderLaunch,
        ) -> Result<Process, SandboxedProviderLaunchError> {
            self.0
                .lock()
                .unwrap()
                .push((request.clone(), launch.clone()));
            Ok(Process)
        }
    }

    fn request() -> SandboxedLaunchRequest {
        SandboxedLaunchRequest {
            lock: RunVersionLock {
                provider: "codex".into(),
                canonical_path: "/private/codex".into(),
                file_identity: "1:2".into(),
                digest_sha256: "a".repeat(64),
                version: "1".into(),
                compatibility_entry: "codex-1".into(),
            },
            profile: SandboxLaunchProfile::new(
                std::path::Path::new("/workspace"),
                &[PathBuf::from("/workspace")],
                &[],
                vec![],
                SandboxNetworkPolicy::Disabled,
                SandboxResourceLimits {
                    max_processes: 1,
                    max_memory_bytes: 1,
                    max_cpu_time_ms: 1,
                },
            )
            .unwrap(),
        }
    }

    fn launch(provider: &str, executable: &str) -> ProviderLaunch {
        ProviderLaunch {
            provider: provider.into(),
            executable: executable.into(),
            arguments: vec![],
            intent: LaunchIntent::Start,
        }
    }

    #[test]
    fn atomic_port_receives_the_bound_profile_with_the_exact_spawn() {
        let port = LaunchPort::default();
        let launcher = SandboxedLauncher::new(request(), port);
        launcher.launch(&launch("codex", "/private/codex")).unwrap();
        let calls = launcher.inner.0.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0.lock.provider, "codex");
        assert_eq!(calls[0].1.executable, PathBuf::from("/private/codex"));
    }

    #[test]
    fn altered_launch_never_reaches_the_atomic_port() {
        let port = LaunchPort::default();
        let launcher = SandboxedLauncher::new(request(), port);
        assert!(matches!(
            launcher.launch(&launch("claude", "/private/claude")),
            Err(SupervisorError::Lock(LockError::ProviderChanged))
        ));
        assert!(launcher.inner.0.lock().unwrap().is_empty());
    }
}
