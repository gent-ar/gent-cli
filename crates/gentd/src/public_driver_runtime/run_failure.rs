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

/// Produces the durable, user-visible explanation for a provider failure.
///
/// A provider can fail before it announces a session ID. The normalized event
/// is then fenced off correctly, so the prompt-dispatch transcript is the
/// only durable place the client can show the actual reason.
pub(crate) fn failure_notice(error: &RuntimeError) -> String {
    format!("{RUN_FAILURE_NOTICE}\n\nReason: {error}")
}

pub(crate) fn failure_fact(error: &RuntimeError) -> PublicWireFact {
    PublicWireFact::Event(NormalizedProviderEvent::ProviderFailure {
        classification: ProviderFailureClassification::Lifecycle,
        message: failure_notice(error),
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
        error: &RuntimeError,
    ) -> Result<(), RuntimeError> {
        let notice = failure_notice(error);
        let code = match error {
            RuntimeError::ProviderRun(error) => error.failure_code(),
            _ => None,
        };
        match self
            .dispatches
            .fail_prelaunch(message_id, coordinator_id, host_epoch, &notice, code)
        {
            Ok(()) => Ok(()),
            Err(error) if is_daemon_fatal(&error) => Err(error),
            Err(_) => tolerate_run_scoped(
                self.ledger
                    .record_agent_chat_prompt_failure_notice(
                        message_id,
                        coordinator_id,
                        host_epoch,
                        &notice,
                        code,
                    )
                    .map_err(RuntimeError::from),
            )
            .map(|_| ()),
        }
    }
}

pub(crate) fn provider_exited(provider: &str, code: Option<i32>) -> RuntimeError {
    let status = code.map_or_else(|| "unknown".into(), |code| code.to_string());
    RuntimeError::ProviderRun(gent_ports::PublicProviderRunError::Failed(format!(
        "{provider} exited before completing the turn (process status {status})."
    )))
}
