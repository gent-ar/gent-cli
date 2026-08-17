//! Daemon-owned provider-effect ingress with durable-source-before-projection ordering.

use gent_core::DecisionEvidence;
use gent_ports::{IngressMode, Ledger, RunProjectionLedger, RunSessionBinding};
use gent_types::{
    Event, HostEpoch, NormalizedLifecycleSignal, NormalizedProviderEvent, ReceiptId, RunLiveStatus,
};
use sha2::{Digest, Sha256};

use crate::{Coordinator, ProviderRunAuthority, RunProjectionService, RuntimeError};

/// One effect emitted by an owned provider runner or private bridge adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderLifecycleEffect {
    SessionStarted { provider_session_id: String },
    Normalized(NormalizedProviderEvent),
    Lifecycle(NormalizedLifecycleSignal),
    ProviderAcknowledged { decision_id: String },
    ProviderSettled { decision_id: String },
    Terminal { reason: String },
}

/// Persists source facts before applying daemon-owned sessions, decisions, or projections.
#[derive(Debug)]
pub struct ProviderLifecycleIngress<L> {
    coordinator: Coordinator<L>,
    projections: RunProjectionService<L>,
    authority: ProviderRunAuthority,
}

impl<L> ProviderLifecycleIngress<L>
where
    L: Clone + Ledger + RunProjectionLedger,
{
    /// Constructs an inert ingress unless public-driver authority is explicitly supplied.
    #[must_use]
    pub fn new(coordinator: Coordinator<L>, authority: ProviderRunAuthority) -> Self {
        Self {
            projections: RunProjectionService::new(coordinator.clone()),
            coordinator,
            authority,
        }
    }

    /// Records one runner-owned effect and returns a status only when it changes a projection.
    ///
    /// The `event_id` must be stable across retries. Its source event is appended before every
    /// follow-up mutation, so a projection can always be reconstructed from durable history.
    ///
    /// # Errors
    /// Returns an error when authority, ownership, durable source persistence, or reduction fails.
    pub fn record(
        &self,
        event_id: String,
        run_id: String,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        effect: ProviderLifecycleEffect,
    ) -> Result<Option<RunLiveStatus>, RuntimeError> {
        self.require_owner(&run_id, coordinator_id, host_epoch, needs_session(&effect))?;
        let source = Self::source_event(event_id, &run_id, host_epoch, &effect)?;
        let source = match self.coordinator.ledger.find_event(&source.event_id)? {
            Some(existing) if same_source(&existing, &source) => existing,
            Some(_) => return Err(invariant("provider lifecycle event id was reused")),
            None => self.coordinator.ledger.append_event(&source)?,
        };
        match effect {
            ProviderLifecycleEffect::SessionStarted {
                provider_session_id,
            } => {
                self.coordinator
                    .ledger
                    .save_run_session_binding(&RunSessionBinding {
                        run_id,
                        provider_session_id,
                    })?;
                Ok(None)
            }
            ProviderLifecycleEffect::Normalized(event) => {
                if let NormalizedProviderEvent::DecisionSettled { decision_id } = &event {
                    self.coordinator
                        .apply_decision_evidence(decision_id, DecisionEvidence::ProviderSettled)?;
                }
                self.projections
                    .record_normalized_event(
                        run_id,
                        coordinator_id,
                        host_epoch,
                        source.cursor,
                        &event,
                    )
                    .map(Some)
            }
            ProviderLifecycleEffect::Lifecycle(signal) => self
                .projections
                .record_lifecycle_signal(run_id, coordinator_id, host_epoch, source.cursor, &signal)
                .map(Some),
            ProviderLifecycleEffect::ProviderAcknowledged { decision_id } => self
                .coordinator
                .apply_decision_evidence(&decision_id, DecisionEvidence::ProviderAcknowledged)
                .map(|_| None),
            ProviderLifecycleEffect::ProviderSettled { decision_id } => self
                .coordinator
                .apply_decision_evidence(&decision_id, DecisionEvidence::ProviderSettled)
                .map(|_| None),
            ProviderLifecycleEffect::Terminal { .. } => Ok(None),
        }
    }

    fn require_owner(
        &self,
        run_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        requires_session: bool,
    ) -> Result<(), RuntimeError> {
        if self.authority != ProviderRunAuthority::PublicDrivers {
            return Err(invariant(
                "observer mode cannot accept provider lifecycle effects",
            ));
        }
        let ingress = self.coordinator.ledger.host_ingress()?;
        if ingress.epoch != host_epoch || ingress.mode == IngressMode::Closed {
            return Err(invariant("provider lifecycle source is fenced or closed"));
        }
        let owned = self
            .coordinator
            .ledger
            .find_run_lease(run_id)?
            .is_some_and(|lease| {
                lease.coordinator_id == coordinator_id && lease.host_epoch == host_epoch
            });
        if !owned {
            return Err(invariant(
                "provider lifecycle reporter does not own the run",
            ));
        }
        if requires_session
            && self
                .coordinator
                .ledger
                .find_run_session_binding(run_id)?
                .is_none()
        {
            return Err(invariant(
                "provider lifecycle requires a daemon-owned session",
            ));
        }
        Ok(())
    }

    fn source_event(
        event_id: String,
        run_id: &str,
        host_epoch: HostEpoch,
        effect: &ProviderLifecycleEffect,
    ) -> Result<Event, RuntimeError> {
        if event_id.trim().is_empty() {
            return Err(invariant("provider lifecycle event id is required"));
        }
        let payload = match effect {
            ProviderLifecycleEffect::SessionStarted {
                provider_session_id,
            } => {
                serde_json::json!({ "runId": run_id, "effect": effect_name(effect), "sessionDigest": digest(provider_session_id) })
            }
            ProviderLifecycleEffect::Normalized(event) => {
                serde_json::json!({ "runId": run_id, "event": event })
            }
            ProviderLifecycleEffect::Lifecycle(signal) => {
                serde_json::json!({ "runId": run_id, "signal": signal })
            }
            ProviderLifecycleEffect::ProviderAcknowledged { decision_id }
            | ProviderLifecycleEffect::ProviderSettled { decision_id } => {
                serde_json::json!({ "runId": run_id, "effect": effect_name(effect), "decisionId": decision_id })
            }
            ProviderLifecycleEffect::Terminal { reason } => {
                serde_json::json!({ "runId": run_id, "effect": effect_name(effect), "reason": reason })
            }
        };
        Ok(Event {
            cursor: 0,
            event_id,
            receipt_id: ReceiptId(format!("provider:{run_id}")),
            host_epoch,
            kind: "providerLifecycle".into(),
            payload,
        })
    }
}

const fn needs_session(effect: &ProviderLifecycleEffect) -> bool {
    !matches!(
        effect,
        ProviderLifecycleEffect::SessionStarted { .. } | ProviderLifecycleEffect::Terminal { .. }
    )
}

const fn effect_name(effect: &ProviderLifecycleEffect) -> &'static str {
    match effect {
        ProviderLifecycleEffect::SessionStarted { .. } => "sessionStarted",
        ProviderLifecycleEffect::Normalized(_) => "normalized",
        ProviderLifecycleEffect::Lifecycle(_) => "lifecycle",
        ProviderLifecycleEffect::ProviderAcknowledged { .. } => "providerAcknowledged",
        ProviderLifecycleEffect::ProviderSettled { .. } => "providerSettled",
        ProviderLifecycleEffect::Terminal { .. } => "terminal",
    }
}

fn invariant(message: &str) -> RuntimeError {
    RuntimeError::Ledger(gent_ports::LedgerError::Invariant(message.into()))
}

fn same_source(existing: &Event, proposed: &Event) -> bool {
    existing.receipt_id == proposed.receipt_id
        && existing.host_epoch == proposed.host_epoch
        && existing.kind == proposed.kind
        && existing.payload == proposed.payload
}

fn digest(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}
