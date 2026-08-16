#![allow(dead_code)] // The shipped daemon remains observer-only until the evidence gates pass.
//! Composition-edge adapter from pure public-driver effects to runtime lifecycle ingress.

use gent_drivers::{SessionEffect, public_protocol::PublicWireFact};
use gent_ports::{Ledger, RunProjectionLedger};
use gent_runtime::{
    Coordinator, ProviderLifecycleEffect, ProviderLifecycleIngress, ProviderRunAuthority,
    RuntimeError,
};
use gent_types::{HostEpoch, RunLiveStatus};

/// Converts public-driver effects at the daemon edge without granting process authority.
///
/// `gentd` is the only crate allowed to know both the driver and runtime domains. The wrapped
/// runtime ingress still rejects every call unless a future authority-gated daemon owns the run.
#[derive(Debug)]
pub(crate) struct ProviderEffectDispatcher<L> {
    ingress: ProviderLifecycleIngress<L>,
}

impl<L> ProviderEffectDispatcher<L>
where
    L: Clone + Ledger + RunProjectionLedger,
{
    /// Builds an inert dispatcher unless public-driver authority is explicitly supplied.
    #[must_use]
    pub(crate) fn new(coordinator: Coordinator<L>, authority: ProviderRunAuthority) -> Self {
        Self {
            ingress: ProviderLifecycleIngress::new(coordinator, authority),
        }
    }

    /// Persists one reduced driver effect through the daemon-owned lifecycle ingress.
    ///
    /// `StartAttempt` is process-local control flow and deliberately never reaches the ledger.
    /// All external facts preserve the caller-supplied stable source event ID.
    pub(crate) fn record(
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

    /// Persists a fact from the documented Claude/Codex public-wire normalizer.
    ///
    /// The normalizer has already discarded unknown provider fields. This composition edge maps
    /// only its typed facts into the same fenced ingress as the generic process reducer.
    pub(crate) fn record_public_wire_fact(
        &self,
        event_id: String,
        run_id: String,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        fact: &PublicWireFact,
    ) -> Result<Option<RunLiveStatus>, RuntimeError> {
        let effect = match fact {
            PublicWireFact::SessionStarted {
                provider_session_id,
            } => ProviderLifecycleEffect::SessionStarted {
                provider_session_id: provider_session_id.clone(),
            },
            PublicWireFact::Event(event) => ProviderLifecycleEffect::Normalized(event.clone()),
            PublicWireFact::Lifecycle(signal) => ProviderLifecycleEffect::Lifecycle(signal.clone()),
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

#[cfg(test)]
mod tests {
    use gent_drivers::{SessionEffect, public_protocol::PublicWireFact};
    use gent_ports::{Ledger, RunLease, RunRecord};
    use gent_runtime::{Coordinator, ProviderRunAuthority};
    use gent_store::SqliteLedger;
    use gent_types::{
        CapabilitySet, EventResume, HostEpoch, NormalizedLifecycleSignal, NormalizedProviderEvent,
        RootActivity,
    };

    use super::ProviderEffectDispatcher;

    fn prepare(ledger: &SqliteLedger) {
        ledger
            .create_run(&RunRecord {
                run_id: "run-a".into(),
                parent_run_id: None,
                provider: "claude".into(),
            })
            .unwrap();
        ledger
            .claim_run_lease(&RunLease {
                run_id: "run-a".into(),
                coordinator_id: "daemon-a".into(),
                host_epoch: HostEpoch(1),
            })
            .unwrap();
    }

    #[test]
    fn dispatcher_persists_driver_facts_but_ignores_process_local_retries() {
        let ledger = SqliteLedger::in_memory().unwrap();
        prepare(&ledger);
        let dispatcher = ProviderEffectDispatcher::new(
            Coordinator::new(ledger.clone(), CapabilitySet::default()),
            ProviderRunAuthority::PublicDrivers,
        );
        assert_eq!(
            dispatcher
                .record(
                    "ignored-retry".into(),
                    "run-a".into(),
                    "daemon-a",
                    HostEpoch(1),
                    &SessionEffect::StartAttempt { attempt: 2 },
                )
                .unwrap(),
            None
        );
        dispatcher
            .record(
                "driver-session".into(),
                "run-a".into(),
                "daemon-a",
                HostEpoch(1),
                &SessionEffect::SessionStarted {
                    provider_session_id: "native-session".into(),
                },
            )
            .unwrap();
        let status = dispatcher
            .record(
                "driver-turn".into(),
                "run-a".into(),
                "daemon-a",
                HostEpoch(1),
                &SessionEffect::Normalized {
                    event: NormalizedProviderEvent::TurnStarted {
                        turn_id: "turn-a".into(),
                    },
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(status.status.snapshot_cursor, 2);
        assert_eq!(
            dispatcher
                .record(
                    "driver-terminal".into(),
                    "run-a".into(),
                    "daemon-a",
                    HostEpoch(1),
                    &SessionEffect::Terminal {
                        reason: "completed".into(),
                    },
                )
                .unwrap(),
            None
        );
        assert_eq!(
            ledger
                .find_run_session_binding("run-a")
                .unwrap()
                .unwrap()
                .provider_session_id,
            "native-session"
        );
        let EventResume::Delta { events } = ledger.resume_events(0).unwrap() else {
            panic!("driver facts must remain cursor-resumable");
        };
        assert_eq!(events.len(), 3);
        assert!(!events[0].payload.to_string().contains("native-session"));
    }

    #[test]
    fn dispatcher_accepts_only_normalized_public_wire_facts() {
        let ledger = SqliteLedger::in_memory().unwrap();
        prepare(&ledger);
        let dispatcher = ProviderEffectDispatcher::new(
            Coordinator::new(ledger.clone(), CapabilitySet::default()),
            ProviderRunAuthority::PublicDrivers,
        );
        dispatcher
            .record_public_wire_fact(
                "public-session".into(),
                "run-a".into(),
                "daemon-a",
                HostEpoch(1),
                &PublicWireFact::SessionStarted {
                    provider_session_id: "provider-private-id".into(),
                },
            )
            .unwrap();
        let status = dispatcher
            .record_public_wire_fact(
                "public-activity".into(),
                "run-a".into(),
                "daemon-a",
                HostEpoch(1),
                &PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootActivity {
                    activity: RootActivity::Generating,
                }),
            )
            .unwrap()
            .unwrap();
        assert_eq!(status.status.snapshot_cursor, 2);
        let source = ledger.find_event("public-session").unwrap().unwrap();
        assert!(!source.payload.to_string().contains("provider-private-id"));
    }
}
