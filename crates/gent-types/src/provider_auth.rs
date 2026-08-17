//! Secret-free provider authentication values for public Claude and Codex login flows.

use serde::{Deserialize, Serialize};

/// The only public provider identities allowed in this authentication contract.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum ProviderAuthProvider {
    Claude,
    Codex,
}

/// A user-selectable authentication route. Credentials never occupy this contract.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum ProviderAuthMethod {
    AccountBrowser,
    DeviceCode,
    ApiKey,
    AccessToken,
}

/// A digest-bound executable identity, captured before login is started.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderAuthBinaryLock {
    /// Daemon-captured canonical executable identity; clients cannot supply a replacement path.
    pub canonical_executable_id: String,
    pub digest_sha256: String,
    pub version: String,
}

/// Public, typed lifecycle state for a provider authentication attempt.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum ProviderAuthLifecycle {
    Unknown,
    Unauthenticated,
    ChallengeOffered,
    OpeningBrowser,
    AwaitingDeviceApproval,
    Verifying,
    Authenticated,
    Expired,
    Cancelled,
    TimedOut,
    ProviderChanged,
    Failed,
}

/// A bounded prompt for a terminal or native client to render as an `askTool` choice.
///
/// Selecting a method may cause a future executor to open an external browser or an interactive
/// provider process. It never authorizes a credential to cross Gent IPC or durable storage.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderAuthChallenge {
    pub challenge_id: String,
    pub provider: ProviderAuthProvider,
    pub binary_lock: ProviderAuthBinaryLock,
    pub methods: Vec<ProviderAuthMethod>,
    pub expires_at_unix_seconds: u64,
}

/// A method-only answer to a prior [`ProviderAuthChallenge`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderAuthMethodSelection {
    pub challenge_id: String,
    pub method: ProviderAuthMethod,
}

/// Status that can be projected to a terminal or native client without exposing credentials.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderAuthStatus {
    pub provider: ProviderAuthProvider,
    pub binary_lock: ProviderAuthBinaryLock,
    pub lifecycle: ProviderAuthLifecycle,
    pub selected_method: Option<ProviderAuthMethod>,
    pub expires_at_unix_seconds: Option<u64>,
}

impl ProviderAuthChallenge {
    /// Validates the fixed, secret-free bounds required before a challenge is rendered.
    ///
    /// # Errors
    /// Returns an error for invalid identifiers, binary locks, or method choices.
    pub fn validate(&self) -> Result<(), ProviderAuthContractError> {
        validate_id(&self.challenge_id)?;
        self.binary_lock.validate()?;
        if self.methods.is_empty() || self.methods.len() > 4 {
            return Err(ProviderAuthContractError::InvalidMethodCount);
        }
        let mut methods = self.methods.clone();
        methods.sort_by_key(|method| *method as u8);
        methods.dedup();
        if methods.len() != self.methods.len() {
            return Err(ProviderAuthContractError::DuplicateMethod);
        }
        Ok(())
    }
}

impl ProviderAuthBinaryLock {
    /// Validates canonical identity, exact SHA-256 digest, and a bounded provider version.
    ///
    /// # Errors
    /// Returns an error when the executable identity, digest, or version is malformed.
    pub fn validate(&self) -> Result<(), ProviderAuthContractError> {
        if self.canonical_executable_id.is_empty()
            || self.canonical_executable_id.len() > 1024
            || self.canonical_executable_id.chars().any(char::is_control)
        {
            return Err(ProviderAuthContractError::InvalidExecutableIdentity);
        }
        if self.digest_sha256.len() != 64
            || !self
                .digest_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(ProviderAuthContractError::InvalidDigest);
        }
        if self.version.is_empty() || self.version.len() > 128 {
            return Err(ProviderAuthContractError::InvalidVersion);
        }
        Ok(())
    }
}

impl ProviderAuthMethodSelection {
    /// Validates the bounded identifier used to correlate a method answer.
    ///
    /// # Errors
    /// Returns an error when the challenge identifier is malformed.
    pub fn validate(&self) -> Result<(), ProviderAuthContractError> {
        validate_id(&self.challenge_id)
    }
}

fn validate_id(value: &str) -> Result<(), ProviderAuthContractError> {
    if value.is_empty() || value.len() > 128 {
        return Err(ProviderAuthContractError::InvalidIdentifier);
    }
    Ok(())
}

/// Contract validation failures deliberately contain no user- or provider-secret values.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ProviderAuthContractError {
    #[error("provider authentication identifier is invalid")]
    InvalidIdentifier,
    #[error("provider authentication executable identity is invalid")]
    InvalidExecutableIdentity,
    #[error("provider authentication digest is invalid")]
    InvalidDigest,
    #[error("provider authentication version is invalid")]
    InvalidVersion,
    #[error("provider authentication method count is invalid")]
    InvalidMethodCount,
    #[error("provider authentication methods must be unique")]
    DuplicateMethod,
}

#[cfg(test)]
mod tests {
    use super::{
        ProviderAuthBinaryLock, ProviderAuthChallenge, ProviderAuthContractError,
        ProviderAuthMethod, ProviderAuthProvider,
    };

    fn lock() -> ProviderAuthBinaryLock {
        ProviderAuthBinaryLock {
            canonical_executable_id: "provider-id:claude".into(),
            digest_sha256: "a".repeat(64),
            version: "1.2.3".into(),
        }
    }

    #[test]
    fn challenge_requires_a_valid_locked_binary_and_methods() {
        let challenge = ProviderAuthChallenge {
            challenge_id: "challenge-1".into(),
            provider: ProviderAuthProvider::Codex,
            binary_lock: lock(),
            methods: vec![ProviderAuthMethod::AccountBrowser],
            expires_at_unix_seconds: 42,
        };
        assert_eq!(challenge.validate(), Ok(()));
    }

    #[test]
    fn malformed_digest_is_rejected_without_echoing_it() {
        let lock = ProviderAuthBinaryLock {
            canonical_executable_id: "provider-id:claude".into(),
            digest_sha256: "not-a-digest".into(),
            version: "1.2.3".into(),
        };
        assert_eq!(
            lock.validate(),
            Err(ProviderAuthContractError::InvalidDigest)
        );
    }
}
