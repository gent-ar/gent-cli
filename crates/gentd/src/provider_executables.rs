use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use gent_drivers::{
    PublicProvider,
    lock::{capture, recheck, rechecked_identity},
};
use gent_ports::ProvisionedProviderLockReader;
use gent_protocol::DependencyProvider;
use gent_types::{AgentChatProvider, RunVersionLock};

use crate::{
    local_provider_locks::{
        LOCAL_ENTRY, LOCAL_VERSION, LocalProviderLockError, LocalProviderLocks,
    },
    private_provider_readiness::PrivateProviderReadiness,
    standalone_authority_release::StandaloneAuthorityRelease,
    standalone_provider_setup::{provider_executable, provider_prefix},
};

#[derive(Clone, Default)]
pub(crate) struct ProviderExecutables {
    claude: Option<PathBuf>,
    codex: Option<PathBuf>,
    installed: Option<InstalledProviders>,
}

#[derive(Clone)]
struct InstalledProviders {
    prefix: PathBuf,
    installations: Arc<dyn ProvisionedProviderLockReader>,
    release: Option<StandaloneAuthorityRelease>,
}

impl std::fmt::Debug for ProviderExecutables {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderExecutables")
            .field("claude", &self.claude)
            .field("codex", &self.codex)
            .field(
                "installed",
                &self.installed.as_ref().map(|installed| &installed.prefix),
            )
            .finish()
    }
}

impl ProviderExecutables {
    #[cfg(test)]
    pub(crate) const fn explicit(claude: Option<PathBuf>, codex: Option<PathBuf>) -> Self {
        Self {
            claude,
            codex,
            installed: None,
        }
    }

    pub(crate) fn standalone(
        claude: Option<PathBuf>,
        codex: Option<PathBuf>,
        data_dir: &Path,
        installations: impl ProvisionedProviderLockReader + 'static,
        release: Option<StandaloneAuthorityRelease>,
    ) -> Self {
        Self {
            claude,
            codex,
            installed: Some(InstalledProviders {
                prefix: provider_prefix(data_dir),
                installations: Arc::new(installations),
                release,
            }),
        }
    }

    pub(crate) fn release(&self) -> Option<&StandaloneAuthorityRelease> {
        self.installed.as_ref()?.release.as_ref()
    }

    pub(crate) fn explicit_path(&self, provider: AgentChatProvider) -> Option<&Path> {
        match provider {
            AgentChatProvider::Claude => self.claude.as_deref(),
            AgentChatProvider::Codex => self.codex.as_deref(),
            AgentChatProvider::Claurst => None,
        }
    }

    pub(crate) fn installed_readiness(
        &self,
        provider: AgentChatProvider,
    ) -> PrivateProviderReadiness {
        let Some(dependency) = dependency(provider) else {
            return PrivateProviderReadiness::ClaurstUnavailable;
        };
        let Some(installed) = &self.installed else {
            return PrivateProviderReadiness::InstallReview;
        };
        match installed
            .installations
            .find_provisioned_provider_installation(dependency.as_str())
        {
            Err(_) => PrivateProviderReadiness::Unavailable,
            Ok(None) => PrivateProviderReadiness::InstallReview,
            Ok(Some(installation)) => installed
                .verify(dependency, &installation.lock.run_lock)
                .map_or(
                    PrivateProviderReadiness::InvalidInstallation,
                    PrivateProviderReadiness::Ready,
                ),
        }
    }

    pub(crate) fn executable(&self, provider: AgentChatProvider) -> Option<PathBuf> {
        if let Some(explicit) = self.explicit_path(provider) {
            return explicit.is_file().then(|| explicit.to_path_buf());
        }
        match self.installed_readiness(provider) {
            PrivateProviderReadiness::Ready(lock) => Some(PathBuf::from(lock.canonical_path)),
            _ => None,
        }
    }

    pub(crate) fn revision(&self, provider: AgentChatProvider) -> Option<String> {
        if let Some(explicit) = self.explicit_path(provider) {
            let metadata = std::fs::metadata(explicit).ok()?;
            return Some(format!(
                "{}:{}:{:?}",
                explicit.display(),
                metadata.len(),
                metadata.modified().ok()?
            ));
        }
        match self.installed_readiness(provider) {
            PrivateProviderReadiness::Ready(lock) => {
                Some(format!("{}:{}", lock.canonical_path, lock.digest_sha256))
            }
            _ => None,
        }
    }

    pub(crate) fn locks(
        &self,
        provider: AgentChatProvider,
    ) -> Result<LocalProviderLocks, LocalProviderLockError> {
        let public = match provider {
            AgentChatProvider::Claude => PublicProvider::Claude,
            AgentChatProvider::Codex => PublicProvider::Codex,
            AgentChatProvider::Claurst => return Err(LocalProviderLockError::PathUnavailable),
        };
        let locks = LocalProviderLocks::new(public, provider, self.clone());
        self.launch_lock(provider).map(|_| locks)
    }

    pub(crate) fn launch_lock(
        &self,
        provider: AgentChatProvider,
    ) -> Result<RunVersionLock, LocalProviderLockError> {
        if let Some(explicit) = self.explicit_path(provider) {
            return capture_explicit(provider, explicit);
        }
        match self.installed_readiness(provider) {
            PrivateProviderReadiness::Ready(lock) => Ok(RunVersionLock {
                version: LOCAL_VERSION.into(),
                compatibility_entry: LOCAL_ENTRY.into(),
                ..lock
            }),
            PrivateProviderReadiness::InvalidInstallation => {
                Err(LocalProviderLockError::InvalidInstallation)
            }
            _ => Err(LocalProviderLockError::PathUnavailable),
        }
    }
}

impl InstalledProviders {
    fn verify(
        &self,
        provider: DependencyProvider,
        durable: &RunVersionLock,
    ) -> Option<RunVersionLock> {
        let current = current_identity(durable)?;
        let expected = provider_executable(&self.prefix, provider)?
            .canonicalize()
            .ok()?;
        if current.provider != provider.as_str() || Path::new(&current.canonical_path) != expected {
            return None;
        }
        let now = crate::startup::unix_seconds();
        let release = self.release.as_ref()?.load(now).ok()?;
        release
            .authorizes_provider(provider.as_str())
            .then_some(())?;
        release.compatibility().authorize_at(&current, now).ok()?;
        Some(current)
    }
}

fn current_identity(durable: &RunVersionLock) -> Option<RunVersionLock> {
    if recheck(durable).is_ok() {
        return Some(durable.clone());
    }
    rechecked_identity(durable).ok()
}

fn capture_explicit(
    provider: AgentChatProvider,
    path: &Path,
) -> Result<RunVersionLock, LocalProviderLockError> {
    let path = path
        .canonicalize()
        .map_err(|_| LocalProviderLockError::PathUnavailable)?;
    if !path.is_file() {
        return Err(LocalProviderLockError::NotAFile);
    }
    let name = dependency(provider).ok_or(LocalProviderLockError::PathUnavailable)?;
    capture(name.as_str(), &path, LOCAL_VERSION, LOCAL_ENTRY)
        .map_err(|_| LocalProviderLockError::Capture)
}

const fn dependency(provider: AgentChatProvider) -> Option<DependencyProvider> {
    match provider {
        AgentChatProvider::Claude => Some(DependencyProvider::Claude),
        AgentChatProvider::Codex => Some(DependencyProvider::Codex),
        AgentChatProvider::Claurst => None,
    }
}

#[cfg(test)]
#[path = "provider_executables_tests.rs"]
mod tests;
