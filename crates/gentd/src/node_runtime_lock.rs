//! Daemon-owned app Node/npm runtime lock for future approved provisioning.
//!
//! The shipped observer never constructs this value. An approved host must
//! recheck it immediately before any package operation.

use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
};

use gent_drivers::{
    NodeReadOnlyHostLauncher,
    installer::NpmGlobalPrefix,
    node_runtime_lock::{NodeRuntimeLock, NodeRuntimeLockError},
    process::SystemLauncher,
};

const NODE_BINARY_ENV: &str = "GENT_NODE_BINARY";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AppNodeRuntimeLock {
    lock: NodeRuntimeLock,
    private_prefix: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum AppNodeRuntimeLockError {
    #[error("Gent's packaged Node runtime is missing; reinstall Gent")]
    MissingNode,
    #[error(transparent)]
    Runtime(#[from] NodeRuntimeLockError),
}

impl AppNodeRuntimeLock {
    pub(crate) fn from_standalone_environment(
        data_dir: &Path,
    ) -> Result<Self, AppNodeRuntimeLockError> {
        Self::capture(standalone_node_binary(), data_dir)
    }

    pub(crate) fn capture(
        node: Option<PathBuf>,
        data_dir: &Path,
    ) -> Result<Self, AppNodeRuntimeLockError> {
        let node = node.ok_or(AppNodeRuntimeLockError::MissingNode)?;
        Ok(Self {
            lock: NodeRuntimeLock::capture(&node)?,
            private_prefix: data_dir.join("providers").join("npm-global"),
        })
    }

    /// Rechecks the exact Node/npm pair before a future approved operation.
    pub(crate) fn recheck(&self) -> Result<(), AppNodeRuntimeLockError> {
        self.lock.recheck().map_err(Into::into)
    }

    /// The digest consumed by signed package-policy verification.
    #[must_use]
    pub(crate) fn node_digest_sha256(&self) -> &str {
        self.lock.node_digest_sha256()
    }

    /// Rechecks and builds the fixed private-prefix installer for one operation.
    ///
    /// # Errors
    /// Returns an error instead of handing a future host a changed runtime.
    pub(crate) fn rechecked_npm_prefix(&self) -> Result<NpmGlobalPrefix, AppNodeRuntimeLockError> {
        self.recheck()?;
        Ok(NpmGlobalPrefix::new(
            self.lock.clone(),
            self.private_prefix.clone(),
        ))
    }

    pub(crate) fn rechecked_lock(&self) -> Result<NodeRuntimeLock, AppNodeRuntimeLockError> {
        self.recheck()?;
        Ok(self.lock.clone())
    }

    /// Rechecks and binds the app Node runtime to one bounded Ask/Plan launcher.
    ///
    /// # Errors
    /// Returns instead of constructing a launcher when the app-supplied runtime changed.
    pub(crate) fn rechecked_read_only_launcher(
        &self,
        output_limit: usize,
    ) -> Result<NodeReadOnlyHostLauncher, AppNodeRuntimeLockError> {
        self.recheck()?;
        Ok(NodeReadOnlyHostLauncher::new(
            self.lock.clone(),
            output_limit,
        ))
    }
}

pub(crate) fn standalone_provider_launcher(output_limit: usize) -> SystemLauncher {
    provider_launcher_from(standalone_node_binary(), output_limit)
}

fn provider_launcher_from(node: Option<PathBuf>, output_limit: usize) -> SystemLauncher {
    node.as_deref().and_then(Path::parent).map_or_else(
        || SystemLauncher::new(output_limit),
        |node_bin| SystemLauncher::with_node_first(output_limit, node_bin.to_path_buf()),
    )
}

pub(crate) fn standalone_node_binary() -> Option<PathBuf> {
    node_binary_from(
        cfg!(debug_assertions),
        env::var_os(NODE_BINARY_ENV),
        env::current_exe()
            .and_then(std::fs::canonicalize)
            .ok()
            .as_deref(),
    )
}

fn node_binary_from(
    development_build: bool,
    development_override: Option<OsString>,
    gentd_executable: Option<&Path>,
) -> Option<PathBuf> {
    development_override
        .filter(|_| development_build)
        .map(PathBuf::from)
        .or_else(|| gentd_executable.map(packaged_node_from_executable))
        .filter(|node| node.is_file())
}

fn packaged_node_from_executable(executable: &Path) -> PathBuf {
    executable
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join("runtime/node/bin")
        .join(node_name())
}

#[cfg(windows)]
const fn node_name() -> &'static str {
    "node.exe"
}

#[cfg(not(windows))]
const fn node_name() -> &'static str {
    "node"
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{AppNodeRuntimeLock, AppNodeRuntimeLockError};

    #[test]
    fn app_runtime_binds_policy_digest_and_private_prefix() {
        let root = tempfile::tempdir().unwrap();
        let node = write_pair(root.path());
        let runtime = AppNodeRuntimeLock::capture(Some(node), &root.path().join("gentd")).unwrap();
        assert_eq!(runtime.node_digest_sha256().len(), 64);
        let install = runtime
            .rechecked_npm_prefix()
            .unwrap()
            .install_archive(std::path::Path::new("/private/verified.tgz"));
        assert!(
            std::path::Path::new(&install.executable)
                .ends_with(std::path::Path::new("bin").join(super::node_name()))
        );
        assert!(install.arguments[0].ends_with("npm-cli.js"));
        assert_eq!(install.arguments[4], "--prefix");
        assert!(
            std::path::Path::new(&install.arguments[5]).ends_with("gentd/providers/npm-global")
        );
        runtime.recheck().unwrap();
    }

    #[test]
    fn changed_app_runtime_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let node = write_pair(root.path());
        let runtime = AppNodeRuntimeLock::capture(Some(node.clone()), root.path()).unwrap();
        fs::write(node, "replacement").unwrap();
        assert!(matches!(
            runtime.rechecked_npm_prefix(),
            Err(AppNodeRuntimeLockError::Runtime(_))
        ));
    }

    #[test]
    fn changed_app_runtime_cannot_create_a_read_only_provider_launcher() {
        let root = tempfile::tempdir().unwrap();
        let node = write_pair(root.path());
        let runtime = AppNodeRuntimeLock::capture(Some(node.clone()), root.path()).unwrap();
        fs::write(node, "replacement").unwrap();
        assert!(matches!(
            runtime.rechecked_read_only_launcher(1024),
            Err(AppNodeRuntimeLockError::Runtime(_))
        ));
    }

    #[test]
    fn development_builds_prefer_the_node_override() {
        let root = tempfile::tempdir().unwrap();
        let explicit = write_pair(&root.path().join("explicit"));
        let packaged = write_pair(&root.path().join("release").join("runtime").join("node"));
        let resolved = super::node_binary_from(
            true,
            Some(explicit.clone().into_os_string()),
            Some(&root.path().join("release/gentd")),
        );
        assert_eq!(resolved, Some(explicit));
        assert!(packaged.exists());
    }

    #[test]
    fn release_builds_ignore_the_node_override_for_the_packaged_runtime() {
        let root = tempfile::tempdir().unwrap();
        let explicit = write_pair(&root.path().join("explicit"));
        let packaged = write_pair(&root.path().join("release").join("runtime").join("node"));
        let resolved = super::node_binary_from(
            false,
            Some(explicit.into_os_string()),
            Some(&root.path().join("release/gentd")),
        );
        assert_eq!(resolved, Some(packaged));
    }

    #[test]
    fn release_builds_never_fall_back_to_the_node_override() {
        let root = tempfile::tempdir().unwrap();
        let explicit = write_pair(&root.path().join("explicit"));
        let resolved = super::node_binary_from(
            false,
            Some(explicit.into_os_string()),
            Some(&root.path().join("release/gentd")),
        );
        assert!(matches!(
            AppNodeRuntimeLock::capture(resolved, root.path()),
            Err(AppNodeRuntimeLockError::MissingNode)
        ));
    }

    #[test]
    fn a_release_signed_for_the_packaged_node_verifies_despite_a_node_override() {
        use crate::ordinary_authority_release::{
            OrdinaryAuthorityReleaseError, SignedOrdinaryAuthorityRelease, fixture,
        };
        let root = tempfile::tempdir().unwrap();
        let packaged = fixture::runtime(&root.path().join("release").join("runtime"));
        fixture::runtime(&root.path().join("app"));
        let app_node = root.path().join("app/node/bin/node");
        fs::write(&app_node, "the app's other node").unwrap();
        let other = AppNodeRuntimeLock::capture(Some(app_node.clone()), root.path()).unwrap();
        let signer = ed25519_dalek::SigningKey::from_bytes(&[11; 32]);
        let authority = root.path().join("ordinary-authority.json");
        let write = |node: &AppNodeRuntimeLock| {
            let release = fixture::release(&signer, node.node_digest_sha256());
            fs::write(&authority, serde_json::to_vec(&release).unwrap()).unwrap();
        };
        let resolved = super::node_binary_from(
            false,
            Some(app_node.into_os_string()),
            Some(&root.path().join("release/gentd")),
        );
        let runtime = AppNodeRuntimeLock::capture(resolved, root.path()).unwrap();
        let load = || {
            SignedOrdinaryAuthorityRelease::load_bound(
                &authority,
                &fixture::root_keys(&signer),
                &runtime,
                10,
            )
        };
        write(&packaged);
        assert!(load().unwrap().authorizes_provider("codex"));
        write(&other);
        assert!(matches!(
            load(),
            Err(OrdinaryAuthorityReleaseError::RuntimeUnverified)
        ));
    }

    #[test]
    fn standalone_runtime_uses_the_packaged_binary_without_path_lookup() {
        let root = tempfile::tempdir().unwrap();
        let data_dir = root.path().join("gentd");
        let node = write_pair(&root.path().join("release").join("runtime").join("node"));
        let resolved =
            super::node_binary_from(false, None, Some(&root.path().join("release/gentd")));
        let runtime = AppNodeRuntimeLock::capture(resolved, &data_dir).unwrap();
        fs::write(node, "replacement").unwrap();
        assert!(matches!(
            runtime.recheck(),
            Err(AppNodeRuntimeLockError::Runtime(_))
        ));
    }

    #[test]
    fn standalone_runtime_prefers_the_installed_release_runtime() {
        let root = tempfile::tempdir().unwrap();
        let release = root.path().join("release");
        let node = write_pair(&release.join("runtime").join("node"));
        let resolved = super::packaged_node_from_executable(&release.join("gentd"));
        assert_eq!(resolved, node);
    }

    #[test]
    fn standalone_runtime_requires_an_explicit_or_packaged_binary() {
        let root = tempfile::tempdir().unwrap();
        let resolved = super::node_binary_from(false, None, Some(&root.path().join("gentd")));
        assert!(matches!(
            AppNodeRuntimeLock::capture(resolved, root.path()),
            Err(AppNodeRuntimeLockError::MissingNode)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn provider_launches_run_env_node_from_the_packaged_runtime() {
        use gent_drivers::{LaunchIntent, ProcessLauncher, ProviderLaunch, ProviderProcess};
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let release = root.path().join("release");
        let node = write_pair(&release.join("runtime").join("node"));
        fs::write(&node, "#!/bin/sh\necho packaged-node\n").unwrap();
        fs::set_permissions(&node, fs::Permissions::from_mode(0o755)).unwrap();
        let launcher = super::provider_launcher_from(
            super::node_binary_from(false, None, Some(&release.join("gentd"))),
            4096,
        );
        let lock =
            gent_drivers::lock::capture("codex", std::path::Path::new("/bin/sh"), "test", "test")
                .unwrap();
        let process = launcher
            .launch(&ProviderLaunch {
                executable: lock.canonical_path.clone().into(),
                lock,
                provider: "codex".into(),
                arguments: vec!["-c".into(), "node && command -v sh".into()],
                intent: LaunchIntent::Start,
                workspace_root: None,
                workspace_access: gent_types::SandboxWorkspaceAccess::ReadOnly,
            })
            .unwrap();
        process.close_stdin().unwrap();
        assert!(process.wait().unwrap().success());
        let output = String::from_utf8(process.output().stdout.bytes).unwrap();
        assert_eq!(output.lines().next(), Some("packaged-node"));
        assert_eq!(output.lines().count(), 2);
    }

    fn write_pair(root: &std::path::Path) -> std::path::PathBuf {
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let node = bin.join(super::node_name());
        fs::write(&node, "node").unwrap();
        fs::write(bin.join(npm_name()), "npm").unwrap();
        let cli = root.join("lib/node_modules/npm/bin");
        fs::create_dir_all(&cli).unwrap();
        fs::write(cli.join("npm-cli.js"), "npm cli").unwrap();
        node
    }

    fn package() -> gent_ports::ApprovedPackageInstall {
        gent_ports::ApprovedPackageInstall {
            provider: "codex".into(),
            package_name: "package".into(),
            version: "1.0.0".into(),
            integrity: "sha512-test".into(),
            package_policy_digest_sha256: "a".repeat(64),
        }
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
