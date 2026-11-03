use std::path::{Path, PathBuf};

use gent_drivers::windows_sandbox::WindowsSandboxLauncher;
use gent_types::RunVersionLock;

const HELPER_SHA256: Option<&str> = option_env!("GENT_WINDOWS_SANDBOX_HELPER_SHA256");

#[derive(Clone, Debug)]
pub(crate) struct PinnedSandboxHelper {
    lock: RunVersionLock,
}

impl PinnedSandboxHelper {
    pub(crate) fn image_lock(&self) -> &RunVersionLock {
        &self.lock
    }

    pub(crate) fn reviewed_policy(
        &self,
        provider: gent_types::AgentChatProvider,
    ) -> Result<crate::windows_external_proxy_policy::ExternalProxyPolicy, String> {
        crate::windows_external_proxy_admission::require(self, provider)
    }

    pub(crate) fn require_reviewed_policy(
        &self,
        provider: gent_types::AgentChatProvider,
    ) -> Result<(), String> {
        let policy = self.reviewed_policy(provider)?;
        crate::windows_external_proxy_admission::require_installed_package(self, &policy)
    }

    pub(crate) fn packaged() -> Result<Self, String> {
        let executable = std::env::current_exe()
            .and_then(std::fs::canonicalize)
            .map_err(|_| "sandbox daemon path unavailable")?;
        Self::load(&executable, HELPER_SHA256)
    }

    fn load(daemon: &Path, expected: Option<&str>) -> Result<Self, String> {
        let expected = expected
            .filter(|digest| {
                digest.len() == 64
                    && digest
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            })
            .ok_or("this daemon build has no reviewed Windows sandbox helper pin")?;
        let path = helper_path(daemon)?;
        let canonical =
            std::fs::canonicalize(&path).map_err(|_| "packaged sandbox helper unavailable")?;
        let daemon_root = daemon
            .parent()
            .and_then(|parent| std::fs::canonicalize(parent).ok())
            .ok_or("sandbox daemon directory unavailable")?;
        let parent = path
            .parent()
            .and_then(|parent| std::fs::canonicalize(parent).ok())
            .ok_or("packaged sandbox helper directory unavailable")?;
        if parent != daemon_root.join("runtime/windows")
            || canonical.parent() != Some(parent.as_path())
        {
            return Err("packaged sandbox helper escaped its fixed runtime directory".into());
        }
        let lock = gent_drivers::lock::capture(
            "windows-sandbox-helper",
            &canonical,
            "1",
            "windows-psec-job-v1",
        )
        .map_err(|_| "packaged sandbox helper identity unavailable")?;
        if lock.digest_sha256 != expected {
            return Err("packaged sandbox helper does not match this daemon build".into());
        }
        Ok(Self { lock })
    }

    pub(crate) fn launcher(&self, output_limit: usize) -> WindowsSandboxLauncher {
        WindowsSandboxLauncher::new(self.lock.clone(), output_limit)
    }

    pub(crate) fn runtime_root(&self) -> Result<&Path, String> {
        Path::new(&self.lock.canonical_path)
            .parent()
            .ok_or_else(|| "packaged sandbox helper directory unavailable".into())
    }
}

fn helper_path(daemon: &Path) -> Result<PathBuf, String> {
    Ok(daemon
        .parent()
        .ok_or("sandbox daemon parent unavailable")?
        .join("runtime/windows/provider-helper.exe"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_mismatched_build_pin_cannot_authenticate_a_helper() {
        let directory = tempfile::tempdir().unwrap();
        let daemon = directory.path().join("gentd.exe");
        let helper = helper_path(&daemon).unwrap();
        std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
        std::fs::write(&helper, "reviewed-helper-image").unwrap();
        assert!(PinnedSandboxHelper::load(&daemon, None).is_err());
        assert!(PinnedSandboxHelper::load(&daemon, Some(&"a".repeat(64))).is_err());
        let lock = gent_drivers::lock::capture(
            "windows-sandbox-helper",
            &helper,
            "1",
            "windows-psec-job-v1",
        )
        .unwrap();
        let pinned = PinnedSandboxHelper::load(&daemon, Some(&lock.digest_sha256)).unwrap();
        assert_eq!(pinned.lock, lock);
        assert!(
            pinned
                .require_reviewed_policy(gent_types::AgentChatProvider::Codex)
                .is_err()
        );
        std::fs::write(&helper, "changed-helper-image").unwrap();
        assert_ne!(
            gent_drivers::lock::rechecked_identity(&pinned.lock).unwrap(),
            pinned.lock
        );
        assert!(PinnedSandboxHelper::load(&daemon, Some(&lock.digest_sha256)).is_err());
    }
}

#[cfg(test)]
#[path = "windows_sandbox_smoke_tests.rs"]
mod smoke;
