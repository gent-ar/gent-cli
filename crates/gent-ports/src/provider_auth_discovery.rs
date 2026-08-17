//! Secret-free, daemon-owned discovery of public provider authentication readiness.

use gent_types::{ProviderAuthBinaryLock, ProviderAuthMethod, ProviderAuthProvider};

/// Authentication state observed from an official provider tool without reading credentials.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderAuthAuthentication {
    Authenticated,
    Unauthenticated,
}

/// One daemon-observed provider authentication capability snapshot.
///
/// The executable lock and supported methods are captured by infrastructure. Clients may render
/// this value, but cannot replace its binary identity or provide credentials through this port.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderAuthDiscovery {
    pub provider: ProviderAuthProvider,
    pub binary_lock: ProviderAuthBinaryLock,
    pub authentication: ProviderAuthAuthentication,
    /// Authentication methods supported by the exact observed provider version.
    pub supported_methods: Vec<ProviderAuthMethod>,
}

impl ProviderAuthDiscovery {
    /// Verifies the bounded, secret-free observation returned by an implementation.
    ///
    /// # Errors
    /// Returns an error when the lock or version-supported method set is malformed.
    pub fn validate(&self) -> Result<(), ProviderAuthDiscoveryError> {
        self.binary_lock
            .validate()
            .map_err(|_| ProviderAuthDiscoveryError::InvalidObservation)?;
        if self.supported_methods.is_empty() || self.supported_methods.len() > 4 {
            return Err(ProviderAuthDiscoveryError::InvalidObservation);
        }
        let mut methods = self.supported_methods.clone();
        methods.sort_by_key(|method| *method as u8);
        methods.dedup();
        if methods.len() != self.supported_methods.len() {
            return Err(ProviderAuthDiscoveryError::InvalidObservation);
        }
        Ok(())
    }
}

/// Controlled outcomes from provider-auth discovery that disclose no credential material.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ProviderAuthDiscoveryError {
    #[error("provider authentication discovery is unavailable")]
    Unavailable,
    #[error("provider executable changed during authentication discovery")]
    ProviderChanged,
    #[error("provider authentication observation is invalid")]
    InvalidObservation,
    #[error("provider authentication discovery failed")]
    Failed,
}

/// Discovers official public-provider authentication readiness from daemon-owned observations.
///
/// Implementations may inspect an already authorized executable, but must not read, return, log,
/// or persist credentials. They report only the version-supported methods for the captured lock.
pub trait ProviderAuthDiscoveryPort: Send + Sync {
    /// Observes authentication readiness for one provider from daemon-owned executable metadata.
    ///
    /// # Errors
    /// Returns a controlled error when the provider is unavailable, changes, or cannot be observed.
    fn discover(
        &self,
        provider: ProviderAuthProvider,
    ) -> Result<ProviderAuthDiscovery, ProviderAuthDiscoveryError>;
}

#[cfg(test)]
mod tests {
    use super::{ProviderAuthAuthentication, ProviderAuthDiscovery, ProviderAuthDiscoveryError};
    use gent_types::{ProviderAuthBinaryLock, ProviderAuthMethod, ProviderAuthProvider};

    fn discovery(methods: Vec<ProviderAuthMethod>) -> ProviderAuthDiscovery {
        ProviderAuthDiscovery {
            provider: ProviderAuthProvider::Claude,
            binary_lock: ProviderAuthBinaryLock {
                canonical_executable_id: "provider-id:claude".into(),
                digest_sha256: "a".repeat(64),
                version: "1.2.3".into(),
            },
            authentication: ProviderAuthAuthentication::Unauthenticated,
            supported_methods: methods,
        }
    }

    #[test]
    fn discovery_keeps_a_locked_binary_and_supported_methods() {
        assert_eq!(
            discovery(vec![ProviderAuthMethod::AccountBrowser]).validate(),
            Ok(())
        );
    }

    #[test]
    fn duplicate_supported_methods_are_rejected() {
        assert_eq!(
            discovery(vec![
                ProviderAuthMethod::DeviceCode,
                ProviderAuthMethod::DeviceCode
            ])
            .validate(),
            Err(ProviderAuthDiscoveryError::InvalidObservation)
        );
    }
}
