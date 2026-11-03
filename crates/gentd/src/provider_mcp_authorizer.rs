//! Borrow genuine live process evidence without the runner's startup mutex.
use super::mcp_permissions::PermissionRelay;
use super::{
    mcp::{Authorizer, Call, Decision, Error},
    run_fence::RunFence,
};
use gent_drivers::ProviderProcess;
use gent_ports::{ConversationLedger, Ledger};
use gent_types::{DurableTurnPhase, PermissionRequest};
use std::sync::{Arc, Mutex, Weak};

pub struct EvidenceGate {
    process: Mutex<Option<Weak<dyn ProviderProcess>>>,
    fence: Arc<RunFence>,
    pub relay: Mutex<PermissionRelay>,
}
impl EvidenceGate {
    pub fn admitted(&self, scope: &gent_ports::HostMcpScope) -> bool {
        if self.scope(&scope.source_sha256, &scope.generation).as_ref() != Some(scope) {
            return false;
        }
        let process = self
            .process
            .lock()
            .ok()
            .and_then(|p| p.as_ref().and_then(Weak::upgrade));
        let Some(process) = process else { return false };
        matches!(process.try_exit_code(), Ok(None))
            && process.sandbox_evidence().is_some_and(|proof| {
                proof.run_id() == self.fence.run_id
                    && proof.permission_binding().is_some_and(|b| {
                        b.run_id() == self.fence.run_id
                            && b.conversation_id() == self.fence.conversation_id
                            && b.policy() == &self.fence.policy
                            && b.host_epoch() == self.fence.host_epoch
                    })
            })
    }
    pub fn new(fence: Arc<RunFence>) -> Arc<Self> {
        Arc::new(Self {
            process: Mutex::new(None),
            fence,
            relay: Mutex::new(PermissionRelay::default()),
        })
    }
    pub fn publish<P: ProviderProcess + 'static>(&self, process: &Arc<P>) -> Result<(), Error> {
        if !self.fence.current() || !matches!(process.try_exit_code(), Ok(None)) {
            return Err(Error::Revoked);
        }
        let proof = process.sandbox_evidence().ok_or(Error::NotReady)?;
        let binding = proof.permission_binding().ok_or(Error::NotReady)?;
        if proof.run_id() != self.fence.run_id
            || binding.run_id() != self.fence.run_id
            || binding.conversation_id() != self.fence.conversation_id
            || binding.policy() != &self.fence.policy
            || binding.host_epoch() != self.fence.host_epoch
        {
            return Err(Error::Denied);
        }
        let mut slot = self.process.lock().map_err(|_| Error::Unprovable)?;
        if slot.is_some() {
            return Err(Error::Protocol);
        }
        let erased: Arc<dyn ProviderProcess> = process.clone();
        *slot = Some(Arc::downgrade(&erased));
        Ok(())
    }
    pub fn active_turn(&self) -> Option<String> {
        if !self.fence.current() {
            return None;
        }
        let turns = self.fence.ledger.list_run_turns(&self.fence.run_id).ok()?;
        let mut active = turns.into_iter().filter(|t| {
            matches!(
                t.phase,
                DurableTurnPhase::Active | DurableTurnPhase::WaitingPermission
            )
        });
        let turn = active.next()?;
        if active.next().is_some() {
            return None;
        }
        Some(turn.turn_id)
    }
    pub fn scope(&self, source_sha256: &str, generation: &str) -> Option<gent_ports::HostMcpScope> {
        let turn = self.active_turn()?;
        let lease = self
            .fence
            .ledger
            .find_run_lease(&self.fence.run_id)
            .ok()??;
        if lease.host_epoch != self.fence.host_epoch {
            return None;
        }
        Some(gent_ports::HostMcpScope {
            conversation_id: self.fence.conversation_id.clone(),
            run_id: self.fence.run_id.clone(),
            turn_id: turn,
            coordinator_id: lease.coordinator_id,
            host_epoch: self.fence.host_epoch,
            policy_id: self.fence.policy.policy_id.clone(),
            source_sha256: source_sha256.into(),
            generation: generation.into(),
        })
    }
}
impl Authorizer for EvidenceGate {
    fn decide(&self, call: &Call<'_>) -> Decision {
        let Some(scope) = call.scope else {
            return Decision::NotReady;
        };
        let turn = &scope.turn_id;
        if self.active_turn().as_ref() != Some(turn)
            || scope.host_epoch != self.fence.host_epoch
            || scope.run_id != self.fence.run_id
            || scope.policy_id != self.fence.policy.policy_id
        {
            return Decision::NotReady;
        }
        let process = self
            .process
            .lock()
            .ok()
            .and_then(|p| p.as_ref().and_then(Weak::upgrade));
        let Some(process) = process else {
            return Decision::NotReady;
        };
        if !matches!(process.try_exit_code(), Ok(None)) {
            return Decision::NotReady;
        }
        let Some(proof) = process.sandbox_evidence() else {
            return Decision::NotReady;
        };
        if proof.run_id() != self.fence.run_id
            || !proof.permission_binding().is_some_and(|b| {
                b.run_id() == self.fence.run_id
                    && b.conversation_id() == self.fence.conversation_id
                    && b.policy() == &self.fence.policy
                    && b.host_epoch() == self.fence.host_epoch
            })
        {
            return Decision::NotReady;
        }
        let Some(tool) = call.tool else {
            return Decision::Deny;
        };
        let name = format!("mcp__{}__{tool}", call.server);
        let args = call
            .params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let request = PermissionRequest::new(
            name.clone(),
            crate::permission_category::for_tool(&name),
            Some(args.clone()),
            None,
        );
        match crate::permission_preflight::evaluate_callback(
            &self.fence.policy,
            &request,
            &self.fence.run_id,
            &self.fence.conversation_id,
            self.fence.host_epoch,
            Some(proof),
            false,
        ) {
            crate::permission_preflight::PermissionPreflight::Allow => Decision::Allow,
            crate::permission_preflight::PermissionPreflight::Deny => Decision::Deny,
            crate::permission_preflight::PermissionPreflight::Ask => {
                // An SDK can_use_tool request has already travelled through the
                // ordinary policy/approval owner before its host response. That
                // response grants only its exact arguments, once, in this turn.
                let permitted = self.relay.lock().is_ok_and(|mut r| {
                    r.permits(turn, call.request_key, &name, &args)
                        .unwrap_or(false)
                });
                if permitted {
                    Decision::Allow
                } else {
                    Decision::Ask
                }
            }
        }
    }
}
