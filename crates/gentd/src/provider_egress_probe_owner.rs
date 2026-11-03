//! Single tracked capture authority. Unknown latches until daemon teardown.
use super::{
    CredentialProbePhase, EgressError, probe_directory::Directory, probe_process::ProbeFailure,
};
use std::sync::{Mutex, MutexGuard, OnceLock};
#[derive(Default)]
pub(super) struct Owner(Mutex<State>);
#[derive(Default)]
struct State {
    retained: Option<Retained>,
}
struct Retained {
    _directory: Directory,
    failure: ProbeFailure,
    primary: Option<CredentialProbePhase>,
}
pub(super) struct Permit<'a>(MutexGuard<'a, State>);
pub(super) fn process_owner() -> &'static Owner {
    static OWNER: OnceLock<Owner> = OnceLock::new();
    OWNER.get_or_init(Owner::default)
}
impl Owner {
    pub fn enter(&self) -> Result<Permit<'_>, EgressError> {
        let state = self
            .0
            .try_lock()
            .map_err(|_| EgressError::CredentialProbe {
                phase: CredentialProbePhase::Busy,
                cleanup_failed: false,
                cleanup_unproven: false,
            })?;
        if let Some(retained) = &state.retained {
            return Err(EgressError::CredentialProbe {
                phase: retained.primary.unwrap_or(if retained.failure.known {
                    CredentialProbePhase::Cleanup
                } else {
                    CredentialProbePhase::ScopeUnknown
                }),
                cleanup_failed: retained.primary.is_some() || retained.failure.known,
                cleanup_unproven: !retained.failure.known,
            });
        }
        Ok(Permit(state))
    }
}
impl Permit<'_> {
    pub fn retain(&mut self, directory: Directory, failure: ProbeFailure) {
        self.retain_primary(directory, failure, None);
    }
    pub fn retain_primary(
        &mut self,
        mut directory: Directory,
        failure: ProbeFailure,
        primary: Option<CredentialProbePhase>,
    ) {
        directory.in_flight();
        assert!(self.0.retained.is_none());
        self.0.retained = Some(Retained {
            _directory: directory,
            failure,
            primary,
        });
    }
}
