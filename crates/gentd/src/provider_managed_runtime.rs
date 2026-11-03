use std::path::{Path, PathBuf};

use gent_drivers::node_runtime_lock::NodeRuntimeLock;
use gent_ports::{ApprovedPackageInstall, PackageInstallPolicy};
use gent_types::{AgentChatProvider, RunVersionLock};

use super::{ProviderExecutables, current_identity, dependency};

#[derive(Clone, Debug)]
pub(crate) struct ManagedProviderRuntime {
    executable: RunVersionLock,
    package: ApprovedPackageInstall,
    #[cfg(windows)]
    package_root: PathBuf,
    node: NodeRuntimeLock,
}

impl ManagedProviderRuntime {
    pub(crate) fn executable(&self) -> &RunVersionLock {
        &self.executable
    }
    pub(crate) fn package(&self) -> &ApprovedPackageInstall {
        &self.package
    }
    #[cfg(windows)]
    pub(crate) fn package_root(&self) -> &Path {
        &self.package_root
    }
    pub(crate) fn node(&self) -> &NodeRuntimeLock {
        &self.node
    }
    pub(crate) fn recheck(&self) -> Result<(), String> {
        gent_drivers::lock::recheck(&self.executable)
            .map_err(|_| "managed provider executable changed")?;
        self.node
            .recheck()
            .map_err(|_| "managed Node/npm runtime changed".into())
    }
}

impl ProviderExecutables {
    pub(crate) fn managed_runtime(
        &self,
        provider: AgentChatProvider,
        launch: &RunVersionLock,
    ) -> Result<ManagedProviderRuntime, String> {
        if self.explicit_path(provider).is_some() {
            return Err("contained launch requires a trusted managed provider installation".into());
        }
        let dependency = dependency(provider).ok_or("provider has no managed package")?;
        let installed = self
            .installed
            .as_ref()
            .ok_or("managed provider installation is unavailable")?;
        let installation = installed
            .installations
            .find_provisioned_provider_installation(dependency.as_str())
            .map_err(|_| "managed installation provenance is unavailable")?
            .ok_or("managed installation provenance is missing")?;
        let release = installed
            .release
            .as_ref()
            .ok_or("managed release authority is unavailable")?;
        let now = crate::startup::unix_seconds();
        let verified = release
            .load(now)
            .map_err(|_| "managed release authority is unverified")?;
        let executable =
            current_identity(&installation.lock.run_lock).ok_or("managed executable changed")?;
        let prefix = installed
            .prefix_for_installation(dependency, &installation)
            .ok_or("managed package prefix does not match its receipt provenance")?;
        let expected = crate::standalone_provider_setup::provider_executable(&prefix, dependency)
            .ok_or("managed provider platform is unsupported")?;
        if !verified.authorizes_provider(dependency.as_str())
            || verified
                .compatibility()
                .authorize_at(&executable, now)
                .is_err()
            || Path::new(&executable.canonical_path) != expected
            || executable.provider != dependency.as_str()
            || (
                launch.provider.as_str(),
                launch.canonical_path.as_str(),
                launch.file_identity.as_str(),
                launch.digest_sha256.as_str(),
            ) != (
                executable.provider.as_str(),
                executable.canonical_path.as_str(),
                executable.file_identity.as_str(),
                executable.digest_sha256.as_str(),
            )
        {
            return Err("managed provider does not match the exact run image".into());
        }
        let package = verified
            .package_policy()
            .approved_package(dependency.as_str(), now)
            .map_err(|_| "managed package is no longer approved")?;
        let node = release
            .runtime()
            .ok_or("managed Node runtime is unavailable")?
            .rechecked_lock()
            .map_err(|_| "managed Node/npm runtime changed")?;
        let provenance = installation.provenance;
        if (
            provenance.package_name.as_str(),
            provenance.package_version.as_str(),
            provenance.package_integrity.as_str(),
            provenance.node_runtime_digest_sha256.as_str(),
        ) != (
            package.package_name.as_str(),
            package.version.as_str(),
            package.integrity.as_str(),
            node.node_digest_sha256(),
        ) || !digest(&provenance.package_policy_digest_sha256)
            || !digest(&provenance.release_artifact_digest_sha256)
            || !digest(&provenance.receipt_fingerprint_sha256)
        {
            return Err("managed package provenance does not match its approved runtime".into());
        }
        let platform = gent_adapters::provider_platform::host_platform()
            .ok_or("managed platform is unsupported")?;
        if !platform.accepts(
            &executable.provider,
            &package.package_name,
            &package.version,
        ) {
            return Err("managed package belongs to another provider platform".into());
        }
        #[cfg(windows)]
        let package_root = prefix.join("node_modules").join(&package.package_name);
        #[cfg(not(windows))]
        let package_root = prefix.join("lib/node_modules").join(&package.package_name);
        if package_root.canonicalize().ok().as_ref() != Some(&package_root) {
            return Err("managed package root is redirected".into());
        }
        let runtime = ManagedProviderRuntime {
            executable: launch.clone(),
            package,
            #[cfg(windows)]
            package_root,
            node,
        };
        runtime.recheck()?;
        Ok(runtime)
    }
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[cfg(test)]
#[path = "provider_managed_runtime_tests.rs"]
mod tests;

#[cfg(target_os = "linux")]
impl ProviderExecutables {
    pub(crate) fn linux_resources(
        &self,
        provider: AgentChatProvider,
        lock: &RunVersionLock,
    ) -> Result<crate::linux_provider_resources::LinuxProviderResources, String> {
        let runtime = self.managed_runtime(provider, lock)?;
        let release = self
            .installed
            .as_ref()
            .and_then(|v| v.release.as_ref())
            .ok_or("Managed Linux release authority is absent")?
            .clone();
        crate::linux_provider_resources::LinuxProviderResources::prepare(runtime, release)
    }
}
