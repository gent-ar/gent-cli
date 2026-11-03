use std::fmt;
use zeroize::Zeroize;

#[derive(Clone, Eq, PartialEq)]
pub struct CredentialKey([u8; 32]);

impl CredentialKey {
    pub fn new(value: [u8; 32]) -> Self {
        Self(value)
    }

    pub fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for CredentialKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CredentialKey([redacted])")
    }
}

pub struct SecretValue(Vec<u8>);

impl SecretValue {
    pub const MAX_BYTES: usize = 256 * 1024;

    pub fn new(mut value: Vec<u8>) -> Result<Self, CredentialStoreError> {
        if value.is_empty() || value.len() > Self::MAX_BYTES {
            value.zeroize();
            return Err(CredentialStoreError::Invalid);
        }
        Ok(Self(value))
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for SecretValue {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue([redacted])")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CredentialStoreError {
    #[error("credential storage is unavailable")]
    Unavailable,
    #[error("credential storage returned invalid data")]
    Invalid,
    #[error("credential write outcome could not be proven")]
    Unprovable,
    #[error("the new credential was not published; the previous value remains selected")]
    NotCommitted,
    #[error("private credential cleanup must finish before another replacement")]
    CleanupRequired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialCommit {
    AlreadySelected,
    Clean,
    PendingCleanup,
}

pub trait CredentialStore: Send + Sync {
    fn read(&self, key: &CredentialKey) -> Result<Option<SecretValue>, CredentialStoreError>;
    fn write(
        &self,
        key: &CredentialKey,
        value: &SecretValue,
    ) -> Result<CredentialCommit, CredentialStoreError>;
    fn erase(&self, key: &CredentialKey) -> Result<CredentialCommit, CredentialStoreError>;
}
