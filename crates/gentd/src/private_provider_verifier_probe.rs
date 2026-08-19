//! Locked runtime version probe for an npm-installed public provider shim.

use std::path::Path;

use gent_drivers::{
    NodeReadOnlyHostLauncher, ProcessLauncher, ProviderLaunch, node_runtime_lock::NodeRuntimeLock,
    supervisor::LaunchIntent,
};
use gent_protocol::DependencyProvider;
use gent_types::{RunVersionLock, SandboxWorkspaceAccess};

const CAPTURE_BYTES: usize = 1025;

pub(crate) trait PrivateVersionProbe: Clone + Send + Sync {
    fn probe(
        &self,
        provider: DependencyProvider,
        lock: &RunVersionLock,
        executable: &Path,
        argument: &str,
    ) -> Result<String, String>;
}

/// Runs only `--version` through the same rechecked locked-Node environment as Ask/Plan turns.
#[derive(Clone, Debug)]
pub(crate) struct LockedNodeVersionProbe {
    runtime: NodeRuntimeLock,
    launcher: NodeReadOnlyHostLauncher,
}

impl LockedNodeVersionProbe {
    #[must_use]
    pub(crate) fn new(runtime: NodeRuntimeLock) -> Self {
        Self {
            launcher: NodeReadOnlyHostLauncher::new(runtime.clone(), CAPTURE_BYTES),
            runtime,
        }
    }
}

impl PrivateVersionProbe for LockedNodeVersionProbe {
    fn probe(
        &self,
        provider: DependencyProvider,
        lock: &RunVersionLock,
        executable: &Path,
        argument: &str,
    ) -> Result<String, String> {
        if argument != "--version" {
            return Err("private provider version argument is invalid".into());
        }
        self.runtime
            .recheck()
            .map_err(|_| "locked app Node runtime changed".to_owned())?;
        let process = self
            .launcher
            .launch(&ProviderLaunch {
                lock: lock.clone(),
                provider: provider.as_str().into(),
                executable: executable.into(),
                arguments: vec![argument.into()],
                intent: LaunchIntent::Start,
                workspace_root: None,
                workspace_access: SandboxWorkspaceAccess::ReadOnly,
            })
            .map_err(|_| "private provider version probe could not start".to_owned())?;
        if !process
            .wait()
            .map_err(|_| "private provider version probe could not finish".to_owned())?
            .success()
        {
            return Err("private provider version probe failed".into());
        }
        self.runtime
            .recheck()
            .map_err(|_| "locked app Node runtime changed".to_owned())?;
        let output = process.output().stdout.bytes;
        let version = String::from_utf8(output)
            .map_err(|_| "private provider version output is invalid".to_owned())?;
        let version = version.trim();
        (!version.is_empty() && version.len() <= 1024)
            .then_some(version.into())
            .ok_or_else(|| "private provider version output is invalid".into())
    }
}
