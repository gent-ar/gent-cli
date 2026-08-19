//! Effect-boundary compatibility authorization for dormant provider authority.
//!
//! The observer daemon never constructs this adapter. An approved private composition supplies
//! both the verified compatibility assessment and its daemon-owned clock.

use gent_ports::{PublicProviderRunError, RunVersionAuthorizer};
use gent_types::RunVersionLock;

use crate::authority_clock::AuthorityClock;
use crate::compatibility_assessment::CompatibilityAssessment;

/// Rechecks one immutable provider lock against the current authority time on every use.
#[derive(Clone, Debug)]
pub(crate) struct FreshCompatibilityAuthorizer<C> {
    assessment: CompatibilityAssessment,
    clock: C,
}

impl<C> FreshCompatibilityAuthorizer<C> {
    /// Binds immutable compatibility evidence to a daemon-owned source of current time.
    #[must_use]
    pub(crate) const fn new(assessment: CompatibilityAssessment, clock: C) -> Self {
        Self { assessment, clock }
    }
}

impl<C: AuthorityClock> RunVersionAuthorizer for FreshCompatibilityAuthorizer<C> {
    fn authorize(&self, lock: &RunVersionLock) -> Result<(), PublicProviderRunError> {
        self.assessment
            .authorize_at(lock, self.clock.now_unix_seconds())
    }
}

#[cfg(test)]
#[path = "fresh_compatibility_authorizer_tests.rs"]
mod tests;
