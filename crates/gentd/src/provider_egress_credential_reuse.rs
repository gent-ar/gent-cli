use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock, PoisonError},
    time::{Duration, Instant},
};

use super::EgressError;

const REUSE_WINDOW: Duration = Duration::from_secs(300);

pub(super) struct CredentialReuse<V>(OnceLock<Mutex<HashMap<String, (Instant, V)>>>);

impl<V: Clone> CredentialReuse<V> {
    pub(super) const fn new() -> Self {
        Self(OnceLock::new())
    }

    pub(super) fn resolve(
        &self,
        key: String,
        valid_for: impl FnOnce(&V) -> Duration,
        resolve: impl FnOnce() -> Result<V, EgressError>,
    ) -> Result<V, EgressError> {
        let entries = self.0.get_or_init(Mutex::default);
        let started = Instant::now();
        if let Some((until, credential)) = entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
            && *until > started
        {
            return Ok(credential.clone());
        }
        let credential = resolve()?;
        let until = started + valid_for(&credential).min(REUSE_WINDOW);
        entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, (until, credential.clone()));
        Ok(credential)
    }
}

#[cfg(test)]
#[path = "provider_egress_credential_reuse_tests.rs"]
mod tests;
