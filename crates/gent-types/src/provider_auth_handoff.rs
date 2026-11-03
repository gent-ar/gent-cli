use crate::{ProviderAuthChallenge, ProviderAuthContractError, ProviderAuthStatus};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderAuthHandoffMethod {
    AccountBrowser,
    DeviceVerification,
}

#[cfg(test)]
#[path = "provider_auth_handoff_tests.rs"]
mod tests;

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ProviderAuthHandoff {
    DeviceVerification {
        verification_url: String,
        user_code: String,
        expires_at_unix_seconds: u64,
    },
}

impl fmt::Debug for ProviderAuthHandoff {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str("DeviceVerification { public instructions: [REDACTED] }")
    }
}

impl ProviderAuthHandoff {
    pub fn validate(&self) -> Result<(), ProviderAuthContractError> {
        let Self::DeviceVerification {
            verification_url,
            user_code,
            ..
        } = self;
        let rest = verification_url
            .strip_prefix("https://")
            .ok_or(ProviderAuthContractError::InvalidIdentifier)?;
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        if verification_url.len() > 2048
            || verification_url
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
            || authority.is_empty()
            || authority.contains('@')
            || authority.contains('\\')
            || !authority.is_ascii()
            || authority.eq_ignore_ascii_case("localhost")
            || user_code.is_empty()
            || user_code.len() > 64
            || !user_code
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            return Err(ProviderAuthContractError::InvalidIdentifier);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderAuthHandoffState {
    pub status: ProviderAuthStatus,
    pub challenge: Option<ProviderAuthChallenge>,
    pub methods: Vec<ProviderAuthHandoffMethod>,
    pub handoff: Option<ProviderAuthHandoff>,
}

impl ProviderAuthHandoffState {
    pub fn validate(&self) -> Result<(), ProviderAuthContractError> {
        if self.methods.len() > 2
            || self
                .methods
                .iter()
                .enumerate()
                .any(|(index, method)| self.methods[..index].contains(method))
        {
            return Err(ProviderAuthContractError::InvalidMethodCount);
        }
        if let Some(lock) = &self.status.binary_lock {
            lock.validate()?;
        }
        if self.status.challenge_id.is_some() && self.challenge.is_none() {
            return Err(ProviderAuthContractError::InvalidIdentifier);
        }
        if let Some(challenge) = &self.challenge {
            challenge.validate()?;
            if self.status.challenge_id.as_deref() != Some(challenge.challenge_id.as_str())
                || self.status.provider != challenge.provider
                || self.status.binary_lock.as_ref() != Some(&challenge.binary_lock)
                || self.status.expires_at_unix_seconds != Some(challenge.expires_at_unix_seconds)
                || !matches!(
                    self.status.lifecycle,
                    crate::ProviderAuthLifecycle::ChallengeOffered
                        | crate::ProviderAuthLifecycle::Verifying
                )
                || self.methods.is_empty()
            {
                return Err(ProviderAuthContractError::InvalidIdentifier);
            }
        }
        if let Some(handoff) = &self.handoff {
            handoff.validate()?;
            let Some(challenge) = &self.challenge else {
                return Err(ProviderAuthContractError::InvalidIdentifier);
            };
            let ProviderAuthHandoff::DeviceVerification {
                expires_at_unix_seconds,
                ..
            } = handoff;
            if self.status.lifecycle != crate::ProviderAuthLifecycle::Verifying
                || *expires_at_unix_seconds != challenge.expires_at_unix_seconds
                || !self
                    .methods
                    .contains(&ProviderAuthHandoffMethod::DeviceVerification)
            {
                return Err(ProviderAuthContractError::InvalidIdentifier);
            }
        }
        Ok(())
    }
}
