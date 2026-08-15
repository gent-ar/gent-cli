//! Dormant adapter from pure driver effects to daemon-owned lifecycle ingress.

use gent_drivers::SessionEffect;
use gent_ports::{Ledger, RunProjectionLedger};
use gent_types::{HostEpoch, RunLiveStatus};

use crate::{
    Coordinator, ProviderLifecycleEffect, ProviderLifecycleIngress, ProviderRunAuthority,
    RuntimeError,
};

/// Owns the only conversion from public-driver session effects to runtime lifecycle facts.
///
/// Constructing this adapter does not launch a process. Its ingress rejects every effect unless
/// an authority-gated daemon owns the run and current epoch.
#[derive(Debug)]
pub struct ProviderEffectDispatcher<L> {
    ingress: ProviderLifecycleIngress<L>,
}

impl<L> ProviderEffectDispatcher<L>
where
    L: Clone + Ledger + RunProjectionLedger,
{
    /// Creates an inert dispatcher unless explicit public-driver authority is supplied.
    #[must_use]
    pub fn new(coordinator: Coordinator<L>, authority: ProviderRunAuthority) -> Self {
        Self {
            ingress: ProviderLifecycleIngress::new(coordinator, authority),
        }
    }

    /// Persists one reduced driver effect through the lifecycle ingress.
    ///
    /// `StartAttempt` is process-local control flow and therefore deliberately has no ledger
    /// effect. All externally observed facts retain the caller's stable source event ID.
    ///
    /// # Errors
    /// Returns an error when lifecycle ingress rejects authority, ownership, or persistence.
    pub fn record(
        &self,
        event_id: String,
        run_id: String,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        effect: &SessionEffect,
    ) -> Result<Option<RunLiveStatus>, RuntimeError> {
        let Some(effect) = lifecycle_effect(effect) else {
            return Ok(None);
        };
        self.ingress
            .record(event_id, run_id, coordinator_id, host_epoch, effect)
    }
}

fn lifecycle_effect(effect: &SessionEffect) -> Option<ProviderLifecycleEffect> {
    match effect {
        SessionEffect::SessionStarted {
            provider_session_id,
        } => Some(ProviderLifecycleEffect::SessionStarted {
            provider_session_id: provider_session_id.clone(),
        }),
        SessionEffect::Normalized { event } => {
            Some(ProviderLifecycleEffect::Normalized(event.clone()))
        }
        SessionEffect::Terminal { reason } => Some(ProviderLifecycleEffect::Terminal {
            reason: reason.clone(),
        }),
        SessionEffect::StartAttempt { .. } => None,
    }
}
