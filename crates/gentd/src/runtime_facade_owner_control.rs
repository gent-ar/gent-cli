use gent_protocol::runtime_owner::RuntimeOwnerFrame;
use gent_protocol::runtime_owner::{
    RuntimeOwnerQuiesceEffect as Effect, RuntimeOwnerQuiesceFailure as Failure,
    RuntimeOwnerQuiesceReason as Reason,
};
use gent_store::SqliteLedger;
use gent_types::HostEpoch;

use super::RuntimeFacade;
use crate::{
    ordinary_lifecycle_cadence::OrdinaryPromptIngress,
    ordinary_lifecycle_control::OrdinaryPromptPermit,
};

#[derive(Clone)]
pub(crate) struct RuntimeOwnerControlPort {
    ingress: OrdinaryPromptIngress<SqliteLedger>,
    ledger: SqliteLedger,
    epoch: HostEpoch,
}

impl RuntimeOwnerControlPort {
    pub(crate) fn acquire_operation(&self) -> Result<OrdinaryPromptPermit, String> {
        self.ingress
            .lifecycle_control()
            .acquire_operation()
            .map_err(|_| "runtime owner is not accepting operations".into())
    }

    #[cfg(test)]
    pub(crate) fn exchange(&self, frame: RuntimeOwnerFrame) -> Result<RuntimeOwnerFrame, String> {
        self.exchange_typed(frame)
            .map_err(|failure| failure.message)
    }

    pub(crate) fn exchange_typed(
        &self,
        frame: RuntimeOwnerFrame,
    ) -> Result<RuntimeOwnerFrame, Failure> {
        if let RuntimeOwnerFrame::RuntimeOwnerIdentity { request_id } = &frame {
            return Ok(crate::runtime_owner_identity::frame(
                request_id.clone(),
                self.epoch,
            ));
        }
        let digest = crate::api::executable_digest()
            .ok_or_else(|| {
                rejected(
                    Reason::IdentityUnavailable,
                    "runtime owner identity is unavailable",
                )
            })?
            .to_owned();
        let process_id = std::process::id();
        match frame {
            RuntimeOwnerFrame::RuntimeOwnerStatus { request_id } => {
                Ok(RuntimeOwnerFrame::RuntimeOwnerState {
                    request_id,
                    host_epoch: self.epoch,
                    process_id,
                    executable_digest_sha256: digest,
                    quiesced: self.ingress.lifecycle_control().is_quiesced(),
                })
            }
            RuntimeOwnerFrame::QuiesceRuntimeOwner {
                request_id,
                expected_host_epoch,
                expected_process_id,
                expected_executable_digest_sha256,
            } => {
                if (
                    expected_host_epoch,
                    expected_process_id,
                    expected_executable_digest_sha256.as_str(),
                ) != (self.epoch, process_id, digest.as_str())
                {
                    return Err(rejected(
                        Reason::IdentityMismatch,
                        "runtime owner identity changed; refresh before requesting repair",
                    ));
                }
                self.ingress.quiesce_idle_owner(&self.ledger, self.epoch)?;
                Ok(RuntimeOwnerFrame::RuntimeOwnerQuiesced {
                    request_id,
                    host_epoch: self.epoch,
                    process_id,
                    executable_digest_sha256: digest,
                })
            }
            _ => Err(rejected(
                Reason::AdmissionUnavailable,
                "runtime owner response frames are server-only",
            )),
        }
    }

    pub(crate) fn acknowledge_quiesce(&self) {
        self.ingress.lifecycle_control().acknowledge_quiesce();
    }
}

fn rejected(reason: Reason, message: &str) -> Failure {
    Failure {
        effect: Effect::NotApplied,
        reason,
        message: message.into(),
    }
}

impl RuntimeFacade {
    pub(super) fn host_epoch(&self) -> Result<HostEpoch, String> {
        self.coordinator
            .status()
            .map(|status| status.host_epoch)
            .map_err(display)
    }
    pub(super) fn owner_control_port(&self) -> Option<RuntimeOwnerControlPort> {
        Some(RuntimeOwnerControlPort {
            ingress: self.ordinary_prompt_ingress.clone()?,
            ledger: self.local_model_events.clone(),
            epoch: self.host_epoch().ok()?,
        })
    }

    pub(super) fn owner_status(&self) -> Result<gent_types::HostStatus, String> {
        crate::api::stamped_status(self.coordinator.status())
    }
}

pub(super) fn display(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
#[path = "runtime_facade_owner_control_tests.rs"]
mod tests;
