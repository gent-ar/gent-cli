//! Locked direct-host launch for ordinary Ask and Plan turns.
//!
//! This is not an OS sandbox and must never serve Agent, Autonomous, or Bypass
//! work. It deliberately narrows the existing bounded system runner to the
//! durable read-only workspace access carried by an Ask/Plan run.

use crate::node_runtime_lock::NodeRuntimeLock;
use crate::process::{SystemLauncher, SystemProcess};
use crate::supervisor::{ProcessLauncher, ProviderLaunch, SupervisorError};
use gent_types::SandboxWorkspaceAccess;

/// Bounded host runner permitted only for ordinary read-only provider turns.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadOnlyHostLauncher {
    inner: SystemLauncher,
}

/// Read-only host launcher that resolves npm provider shims through one locked Node runtime.
///
/// It is an uncomposed infrastructure seam. Agent, Autonomous, and Bypass modes still require
/// the separate atomic containment-and-spawn port.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeReadOnlyHostLauncher {
    runtime: NodeRuntimeLock,
    inner: SystemLauncher,
}

impl NodeReadOnlyHostLauncher {
    /// Binds the rechecked app Node runtime to a bounded Ask/Plan launcher.
    ///
    /// # Panics
    /// Panics only when a captured Node path has no parent directory.
    #[must_use]
    pub fn new(runtime: NodeRuntimeLock, output_limit: usize) -> Self {
        let node_bin = runtime
            .node_path()
            .parent()
            .expect("captured Node path has a parent")
            .into();
        Self {
            runtime,
            inner: SystemLauncher::with_preferred_node(output_limit, node_bin),
        }
    }
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
        require_read_only(launch)?;
        self.inner.launch(launch)
    }
}

impl ProcessLauncher for NodeReadOnlyHostLauncher {
    type Process = SystemProcess;

    fn launch(&self, launch: &ProviderLaunch) -> Result<SystemProcess, SupervisorError> {
        require_read_only(launch)?;
        self.runtime
            .recheck()
            .map_err(|_| SupervisorError::Launch("locked app Node runtime changed".into()))?;
        self.inner.launch(launch)
    }
}

fn require_read_only(launch: &ProviderLaunch) -> Result<(), SupervisorError> {
    (launch.workspace_access == SandboxWorkspaceAccess::ReadOnly)
        .then_some(())
        .ok_or_else(|| {
            SupervisorError::Launch(
                "direct host launch requires an Ask or Plan read-only selection".into(),
            )
        })
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

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

    #[test]
    fn changed_locked_node_is_refused_before_the_provider_can_start() {
        let root = tempfile::tempdir().unwrap();
        let runtime = runtime(root.path());
        let provider = root.path().join("codex");
        fs::write(&provider, "provider").unwrap();
        let lock = crate::lock::capture("codex", &provider, "1", "entry").unwrap();
        fs::write(root.path().join("bin/node"), "changed node").unwrap();
        let launcher = NodeReadOnlyHostLauncher::new(runtime, 1);
        let error = launcher
            .launch(&ProviderLaunch {
                lock,
                provider: "codex".into(),
                executable: provider,
                arguments: vec![],
                intent: LaunchIntent::Start,
                workspace_root: None,
                workspace_access: SandboxWorkspaceAccess::ReadOnly,
            })
            .unwrap_err();
        assert!(
            matches!(error, SupervisorError::Launch(message) if message.contains("Node runtime"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn npm_style_shims_resolve_the_locked_node_before_host_path_entries() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let runtime = runtime(root.path());
        let node = root.path().join("bin/node");
        fs::set_permissions(&node, fs::Permissions::from_mode(0o700)).unwrap();
        let provider = root.path().join("codex");
        fs::write(&provider, "#!/bin/sh\ncommand -v node\n").unwrap();
        fs::set_permissions(&provider, fs::Permissions::from_mode(0o700)).unwrap();
        let lock = crate::lock::capture("codex", &provider, "1", "entry").unwrap();
        let provider = provider.canonicalize().unwrap();
        let launcher = NodeReadOnlyHostLauncher::new(runtime, 1024);
        let process = launcher
            .launch(&ProviderLaunch {
                lock,
                provider: "codex".into(),
                executable: provider,
                arguments: vec![],
                intent: LaunchIntent::Start,
                workspace_root: None,
                workspace_access: SandboxWorkspaceAccess::ReadOnly,
            })
            .unwrap();
        assert!(process.wait().unwrap().success());
        assert_eq!(
            String::from_utf8(process.output().stdout.bytes)
                .unwrap()
                .trim(),
            node.canonicalize().unwrap().display().to_string()
        );
    }

    fn runtime(root: &std::path::Path) -> NodeRuntimeLock {
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("node"), "node").unwrap();
        fs::write(bin.join(npm_name()), "npm").unwrap();
        let cli = root.join("lib/node_modules/npm/bin");
        fs::create_dir_all(&cli).unwrap();
        fs::write(cli.join("npm-cli.js"), "npm cli").unwrap();
        NodeRuntimeLock::capture(&bin.join("node")).unwrap()
    }

    #[cfg(windows)]
    const fn npm_name() -> &'static str {
        "npm.cmd"
    }

    #[cfg(not(windows))]
    const fn npm_name() -> &'static str {
        "npm"
    }
}
