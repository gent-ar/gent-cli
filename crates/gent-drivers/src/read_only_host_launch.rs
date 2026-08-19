//! Locked direct-host launch for ordinary Ask and Plan turns.
//!
//! This is not an OS sandbox and must never serve Agent, Autonomous, or Bypass
//! work. It deliberately narrows the existing bounded system runner to the
//! durable read-only workspace access carried by an Ask/Plan run.

use crate::process::{SystemLauncher, SystemProcess};
use crate::supervisor::{ProcessLauncher, ProviderLaunch, SupervisorError};
use gent_types::SandboxWorkspaceAccess;

/// Bounded host runner permitted only for ordinary read-only provider turns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadOnlyHostLauncher {
    inner: SystemLauncher,
}

impl ReadOnlyHostLauncher {
    /// Creates a direct host launcher with one fixed per-stream capture limit.
    #[must_use]
    pub const fn new(output_limit: usize) -> Self {
        Self {
            inner: SystemLauncher::new(output_limit),
        }
    }
}

impl ProcessLauncher for ReadOnlyHostLauncher {
    type Process = SystemProcess;

    fn launch(&self, launch: &ProviderLaunch) -> Result<SystemProcess, SupervisorError> {
        (launch.workspace_access == SandboxWorkspaceAccess::ReadOnly)
            .then_some(())
            .ok_or_else(|| {
                SupervisorError::Launch(
                    "direct host launch requires an Ask or Plan read-only selection".into(),
                )
            })?;
        self.inner.launch(launch)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gent_types::{RunVersionLock, SandboxWorkspaceAccess};

    use super::*;
    use crate::supervisor::LaunchIntent;

    #[test]
    fn read_write_launch_is_rejected_before_the_system_runner() {
        let launcher = ReadOnlyHostLauncher::new(1);
        let error = launcher.launch(&ProviderLaunch {
            lock: RunVersionLock {
                provider: "codex".into(),
                canonical_path: "/does-not-exist".into(),
                file_identity: "1:2".into(),
                digest_sha256: "a".repeat(64),
                version: "1".into(),
                compatibility_entry: "entry".into(),
            },
            provider: "codex".into(),
            executable: PathBuf::from("/does-not-exist"),
            arguments: vec![],
            intent: LaunchIntent::Start,
            workspace_root: None,
            workspace_access: SandboxWorkspaceAccess::ReadWrite,
        });
        assert!(
            matches!(error, Err(SupervisorError::Launch(message)) if message.contains("read-only"))
        );
    }
}
