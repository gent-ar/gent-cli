use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use gent_types::{ProviderAuthLifecycle, ProviderAuthProvider, ProviderAuthStatus};

use super::provider_auth_process::{authentication, identified_lock};
use super::provider_auth_session::ProviderAuthSession;
use super::{ProviderAuthPort, StandaloneProviderAuthPort, recover};
use crate::provider_launch_budget::{ProbeRetry, ProviderLaunchError};

const SETTLED_POLL: Duration = Duration::from_millis(20);

pub(super) enum ProviderAuthCheck {
    Running,
    Settled(ProviderAuthOutcome),
}

pub(super) enum ProviderAuthOutcome {
    Session(Box<ProviderAuthSession>),
    Unlocked(ProviderAuthLifecycle),
}

pub(super) type ProviderAuthChecks = Arc<Mutex<Vec<(ProviderAuthProvider, ProviderAuthCheck)>>>;

impl StandaloneProviderAuthPort {
    pub(super) fn checked_status(&self, provider: ProviderAuthProvider) -> ProviderAuthStatus {
        if let Some(status) = self.session_status(provider, true) {
            return status;
        }
        let Some(executable) = self.existing_executable(provider) else {
            return unlocked(provider, ProviderAuthLifecycle::NotInstalled);
        };
        self.start_check(provider, executable);
        let deadline = Instant::now() + self.answer_window;
        loop {
            match self.take_settled(provider) {
                Some(status) => return status,
                None if Instant::now() >= deadline => {
                    return unlocked(provider, ProviderAuthLifecycle::Checking);
                }
                None => thread::sleep(SETTLED_POLL),
            }
        }
    }

    pub(super) fn settled_status(
        &self,
        provider: ProviderAuthProvider,
    ) -> Option<ProviderAuthStatus> {
        if let Some(status) = self.session_status(provider, false) {
            return Some(status);
        }
        let Some(executable) = self.existing_executable(provider) else {
            return Some(unlocked(provider, ProviderAuthLifecycle::NotInstalled));
        };
        self.start_check(provider, executable);
        self.take_settled(provider)
    }

    fn session_status(
        &self,
        provider: ProviderAuthProvider,
        refreshed: bool,
    ) -> Option<ProviderAuthStatus> {
        let mut sessions = recover(&self.sessions);
        let session = sessions.iter_mut().find(|item| item.provider == provider)?;
        if refreshed {
            super::refresh(session, self.probe_timeout);
        }
        Some(session.status())
    }

    fn start_check(&self, provider: ProviderAuthProvider, executable: PathBuf) {
        let Some(control) = self.lifecycle_control.clone() else {
            return;
        };
        let mut checks = recover(&self.checks);
        if checks.iter().any(|(checked, _)| *checked == provider) {
            return;
        }
        checks.push((provider, ProviderAuthCheck::Running));
        let checks = Arc::clone(&self.checks);
        let (timeout, retry) = (self.probe_timeout, self.probe_retry);
        thread::spawn(move || {
            let outcome = probe(provider, &executable, timeout, retry, &control);
            let mut checks = recover(&checks);
            let index = checks.iter().position(|(checked, _)| *checked == provider);
            match (index, outcome) {
                (Some(index), Some(outcome)) => {
                    checks[index].1 = ProviderAuthCheck::Settled(outcome);
                }
                (Some(index), None) => {
                    checks.remove(index);
                }
                (None, _) => {}
            }
        });
    }

    fn take_settled(&self, provider: ProviderAuthProvider) -> Option<ProviderAuthStatus> {
        let mut checks = recover(&self.checks);
        let Some(index) = checks.iter().position(|(checked, _)| *checked == provider) else {
            return Some(
                self.session_status(provider, false)
                    .unwrap_or_else(|| unlocked(provider, ProviderAuthLifecycle::Checking)),
            );
        };
        if matches!(checks[index].1, ProviderAuthCheck::Running) {
            return None;
        }
        let (_, ProviderAuthCheck::Settled(outcome)) = checks.remove(index) else {
            return None;
        };
        Some(match outcome {
            ProviderAuthOutcome::Unlocked(lifecycle) => unlocked(provider, lifecycle),
            ProviderAuthOutcome::Session(session) => {
                let mut sessions = recover(&self.sessions);
                if let Some(existing) = sessions.iter().find(|item| item.provider == provider) {
                    existing.status()
                } else {
                    let status = session.status();
                    sessions.push(*session);
                    status
                }
            }
        })
    }
}

fn probe(
    provider: ProviderAuthProvider,
    executable: &std::path::Path,
    timeout: Duration,
    retry: ProbeRetry,
    control: &crate::ordinary_lifecycle_control::OrdinaryLifecycleControl,
) -> Option<ProviderAuthOutcome> {
    for attempt in 1..=retry.attempts {
        match probe_once(provider, executable, timeout, control) {
            Ok(outcome) => return Some(outcome),
            Err(_) if attempt < retry.attempts => thread::sleep(retry.delay),
            Err(_) => {}
        }
    }
    None
}

fn probe_once(
    provider: ProviderAuthProvider,
    executable: &std::path::Path,
    timeout: Duration,
    control: &crate::ordinary_lifecycle_control::OrdinaryLifecycleControl,
) -> Result<ProviderAuthOutcome, ProviderLaunchError> {
    let lock = match identified_lock(provider, executable, timeout, control) {
        Ok(lock) => lock,
        Err(timed_out @ ProviderLaunchError::TimedOut(_)) => return Err(timed_out),
        Err(ProviderLaunchError::Failed(_)) => {
            return Ok(ProviderAuthOutcome::Unlocked(ProviderAuthLifecycle::Failed));
        }
    };
    let lifecycle = authentication(provider, &lock, timeout, control)?;
    Ok(ProviderAuthOutcome::Session(Box::new(
        ProviderAuthSession::new(provider, lock, lifecycle, Some(control.clone())),
    )))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LaunchAdmission {
    Admitted,
    Unknown,
    SignedOut,
}

pub(crate) fn launch_admission(
    auth: &dyn ProviderAuthPort,
    provider: ProviderAuthProvider,
) -> LaunchAdmission {
    match auth.settled_lifecycle(provider) {
        Some(ProviderAuthLifecycle::Authenticated) => LaunchAdmission::Admitted,
        None | Some(ProviderAuthLifecycle::Checking) => LaunchAdmission::Unknown,
        Some(_) => LaunchAdmission::SignedOut,
    }
}

pub(super) fn unlocked(
    provider: ProviderAuthProvider,
    lifecycle: ProviderAuthLifecycle,
) -> ProviderAuthStatus {
    ProviderAuthStatus {
        challenge_id: None,
        provider,
        binary_lock: None,
        lifecycle,
        selected_method: None,
        expires_at_unix_seconds: None,
    }
}

#[cfg(all(test, unix))]
#[path = "provider_auth_check_tests.rs"]
mod tests;
