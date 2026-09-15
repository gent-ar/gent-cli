use gent_drivers::PublicProvider;
use gent_ports::{PublicProviderResolver, PublicProviderRunError};
use gent_types::{AgentChatProvider, RunVersionLock};

use crate::provider_executables::ProviderExecutables;

pub(crate) const LOCAL_VERSION: &str = "local-unprobed";
pub(crate) const LOCAL_ENTRY: &str = "standalone-local-v1";

#[derive(Clone, Debug)]
pub(crate) struct LocalProviderLocks {
    public: PublicProvider,
    provider: AgentChatProvider,
    executables: ProviderExecutables,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum LocalProviderLockError {
    #[error("local provider path is unavailable")]
    PathUnavailable,
    #[error("local provider path is not a file")]
    NotAFile,
    #[error("local provider identity cannot be captured")]
    Capture,
    #[error("installed provider failed verification and must be reinstalled")]
    InvalidInstallation,
}

impl LocalProviderLocks {
    pub(crate) const fn new(
        public: PublicProvider,
        provider: AgentChatProvider,
        executables: ProviderExecutables,
    ) -> Self {
        Self {
            public,
            provider,
            executables,
        }
    }
}

impl PublicProviderResolver for LocalProviderLocks {
    fn resolve(&self, provider: &str) -> Result<RunVersionLock, PublicProviderRunError> {
        if provider != self.public.executable_name() {
            return Err(PublicProviderRunError::CompatibilityDenied);
        }
        self.executables
            .launch_lock(self.provider)
            .map_err(|_| PublicProviderRunError::CompatibilityDenied)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use gent_ports::PublicProviderResolver;
    use gent_types::AgentChatProvider;

    use crate::provider_executables::ProviderExecutables;

    #[test]
    fn an_explicit_path_is_recaptured_for_every_launch_without_path_discovery() {
        let directory = tempfile::tempdir().unwrap();
        let claude = directory.path().join("chosen-claude");
        fs::write(&claude, "claude executable").unwrap();
        let locks = ProviderExecutables::explicit(Some(claude.clone()), None)
            .locks(AgentChatProvider::Claude)
            .unwrap();

        let lock = locks.resolve("claude").unwrap();
        assert_eq!(lock.provider, "claude");
        assert_eq!(lock.version, "local-unprobed");
        assert_eq!(lock.compatibility_entry, "standalone-local-v1");
        assert!(locks.resolve("codex").is_err());

        fs::write(&claude, "rebuilt developer claude").unwrap();
        assert_ne!(
            locks.resolve("claude").unwrap().digest_sha256,
            lock.digest_sha256
        );
        fs::remove_file(&claude).unwrap();
        assert!(locks.resolve("claude").is_err());
    }
}
