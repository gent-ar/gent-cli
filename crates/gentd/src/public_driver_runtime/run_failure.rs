use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{AgentChatPromptDispatchLedger, LedgerError};
use gent_runtime::RuntimeError;
use gent_types::{HostEpoch, NormalizedProviderEvent, ProviderFailureClassification};

use super::PublicDriversRuntime;

pub(crate) const RUN_FAILURE_NOTICE: &str = "Gent stopped this turn because its provider session failed. Other conversations are unaffected.";

pub(crate) const fn is_daemon_fatal(error: &RuntimeError) -> bool {
    matches!(
        error,
        RuntimeError::Ledger(
            LedgerError::Storage(_)
                | LedgerError::StaleEpoch { .. }
                | LedgerError::IngressClosed { .. }
        )
    )
}

pub(crate) fn failure_fact() -> PublicWireFact {
    PublicWireFact::Event(NormalizedProviderEvent::ProviderFailure {
        classification: ProviderFailureClassification::Lifecycle,
        message: RUN_FAILURE_NOTICE.into(),
    })
}

pub(crate) fn tolerate_run_scoped<T>(
    step: Result<T, RuntimeError>,
) -> Result<Option<T>, RuntimeError> {
    match step {
        Ok(value) => Ok(Some(value)),
        Err(error) if is_daemon_fatal(&error) => Err(error),
        Err(_) => Ok(None),
    }
}

impl<L: AgentChatPromptDispatchLedger, D, R> PublicDriversRuntime<L, D, R> {
    pub(crate) fn fail_claimed_prompt(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        tolerate_run_scoped(self.dispatches.fail_prelaunch(
            message_id,
            coordinator_id,
            host_epoch,
            RUN_FAILURE_NOTICE,
        ))
        .map(|_| ())
    }
}
