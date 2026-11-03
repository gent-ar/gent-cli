use gent_protocol::runtime_owner::{
    RuntimeOwnerQuiesceEffect as Effect, RuntimeOwnerQuiesceFailure as Failure,
    RuntimeOwnerQuiesceReason as Reason,
};
use gent_store::SqliteLedger;
use gent_types::HostEpoch;

use super::OrdinaryPromptIngress;
use crate::ordinary_lifecycle_control::OrdinaryLifecycleControl;

impl<L> OrdinaryPromptIngress<L> {
    pub(crate) fn lifecycle_control(&self) -> OrdinaryLifecycleControl {
        self.control.clone()
    }

    pub(crate) fn quiesce_idle_owner(
        &self,
        ledger: &SqliteLedger,
        epoch: HostEpoch,
    ) -> Result<(), Failure> {
        let entered_ledger = std::cell::Cell::new(false);
        let busy = std::cell::Cell::new(false);
        let result = self.control.quiesce_if_idle(|| {
            let router = self.wake.router.try_lock().map_err(|_| {
                busy.set(true);
                "provider owner is busy"
            })?;
            let claurst = self.async_claurst.try_lock().map_err(|_| {
                busy.set(true);
                "local provider owner is busy"
            })?;
            if !router.is_idle() || claurst.as_ref().is_some_and(|host| !host.is_idle()) {
                busy.set(true);
                return Err("provider owner has active work".into());
            }
            entered_ledger.set(true);
            ledger
                .quiesce_idle_owner(epoch)
                .map_err(|error| error.to_string())
        });
        result.map_err(|message| Failure {
            effect: if entered_ledger.get() {
                Effect::MayHaveChanged
            } else {
                Effect::NotApplied
            },
            reason: if entered_ledger.get() {
                Reason::LedgerOrCommitUnavailable
            } else if busy.get() {
                Reason::Busy
            } else {
                Reason::AdmissionUnavailable
            },
            message,
        })
    }
}
