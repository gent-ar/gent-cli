use std::{
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use gent_core::{ProviderAuthEvent, ProviderAuthState, reduce_provider_auth};
use gent_drivers::{
    interrupt::{ProcessTreeControl, ProcessTreeSignal},
    supervisor::ProviderProcess,
};
use gent_types::{ProviderAuthLifecycle, ProviderAuthProvider, ProviderAuthStatus, RunVersionLock};

use super::provider_auth_codex::CodexDeviceLogin;
use super::provider_auth_process::AuthProcess;
use super::provider_auth_process::{authentication, now, public_lock};
use super::recover;

pub(super) struct ProviderAuthSession {
    pub(super) provider: ProviderAuthProvider,
    pub(super) state: ProviderAuthState,
    pub(super) lock: RunVersionLock,
    pub(super) login: Option<LoginProcess>,
    pub(super) device_login: Option<CodexDeviceLogin>,
    pub(super) lifecycle_control:
        Option<crate::ordinary_lifecycle_control::OrdinaryLifecycleControl>,
}

pub(super) struct LoginProcess {
    pub(super) process: AuthProcess,
    pub(super) deadline: Instant,
    pub(super) next_auth_check: Instant,
}

impl ProviderAuthSession {
    pub(super) fn new(
        provider: ProviderAuthProvider,
        lock: RunVersionLock,
        lifecycle: ProviderAuthLifecycle,
        lifecycle_control: Option<crate::ordinary_lifecycle_control::OrdinaryLifecycleControl>,
    ) -> Self {
        let status = ProviderAuthStatus {
            challenge_id: None,
            provider,
            binary_lock: Some(public_lock(provider, &lock)),
            lifecycle,
            selected_method: None,
            expires_at_unix_seconds: None,
        };
        Self {
            provider,
            state: ProviderAuthState {
                challenge: None,
                status: Some(status),
            },
            lock,
            login: None,
            device_login: None,
            lifecycle_control,
        }
    }

    pub(super) fn challenge_id(&self) -> Option<&str> {
        self.state
            .challenge
            .as_ref()
            .map(|value| value.challenge_id.as_str())
    }

    pub(super) fn status(&self) -> ProviderAuthStatus {
        let mut status = self.state.status.clone().unwrap_or(ProviderAuthStatus {
            challenge_id: None,
            provider: self.provider,
            binary_lock: Some(public_lock(self.provider, &self.lock)),
            lifecycle: ProviderAuthLifecycle::Unauthenticated,
            selected_method: None,
            expires_at_unix_seconds: None,
        });
        status.challenge_id = matches!(
            status.lifecycle,
            ProviderAuthLifecycle::ChallengeOffered | ProviderAuthLifecycle::Verifying
        )
        .then(|| self.challenge_id().map(str::to_owned))
        .flatten();
        status
    }

    pub(super) fn settle(&mut self, lifecycle: ProviderAuthLifecycle) {
        self.device_login = None;
        if let Some(login) = self.login.take() {
            let _ = login.process.signal_tree(ProcessTreeSignal::Kill);
        }
        let mut status = self.status();
        status.lifecycle = lifecycle;
        status.challenge_id = None;
        self.state.challenge = None;
        self.state.status = Some(status);
    }
}

pub(super) fn refresh(session: &mut ProviderAuthSession, probe_timeout: Duration) {
    if let Some(login) = session.device_login.as_mut() {
        let changed = gent_drivers::lock::rechecked_identity(&session.lock)
            .ok()
            .as_ref()
            != Some(&session.lock);
        let result = if changed {
            Some(ProviderAuthLifecycle::ProviderChanged)
        } else {
            login.refresh()
        };
        if let Some(lifecycle) = result {
            session.settle(lifecycle);
        }
        return;
    }
    if let Some(login) = session.login.as_ref() {
        match login.process.try_exit_code() {
            Ok(Some(Some(0))) => {
                let result = auth_probe(session, probe_timeout);
                if let Some(lifecycle) =
                    completed_login_lifecycle(result, Instant::now() >= login.deadline)
                {
                    session.login = None;
                    session.settle(lifecycle);
                }
            }
            Ok(Some(_)) | Err(_) => {
                session.settle(ProviderAuthLifecycle::Failed);
            }
            Ok(None) if Instant::now() >= login.deadline => {
                session.settle(ProviderAuthLifecycle::TimedOut);
            }
            Ok(None) if Instant::now() >= login.next_auth_check => {
                let result = auth_probe(session, probe_timeout);
                if let Some(lifecycle) =
                    live_login_lifecycle(result, Instant::now() >= login.deadline)
                {
                    session.settle(lifecycle);
                } else if let Some(login) = session.login.as_mut() {
                    login.next_auth_check = Instant::now() + Duration::from_secs(2);
                }
            }
            Ok(None) => {}
        }
        return;
    }
    match session.status().lifecycle {
        ProviderAuthLifecycle::ChallengeOffered => {
            session.state = reduce_provider_auth(
                session.state.clone(),
                ProviderAuthEvent::Timeout { now: now() },
            )
            .0;
        }
        lifecycle @ (ProviderAuthLifecycle::Authenticated
        | ProviderAuthLifecycle::Unauthenticated) => reprobe(session, lifecycle, probe_timeout),
        ProviderAuthLifecycle::Failed if session.state.challenge.is_none() => {
            reprobe(session, ProviderAuthLifecycle::Failed, probe_timeout);
        }
        _ => {}
    }
}

pub(super) fn completed_login_lifecycle(
    result: Result<ProviderAuthLifecycle, crate::provider_launch_budget::ProviderLaunchError>,
    deadline_reached: bool,
) -> Option<ProviderAuthLifecycle> {
    match result {
        Ok(ProviderAuthLifecycle::Authenticated) => Some(ProviderAuthLifecycle::Authenticated),
        Ok(_) => Some(ProviderAuthLifecycle::Failed),
        Err(_) if deadline_reached => Some(ProviderAuthLifecycle::TimedOut),
        Err(_) => None,
    }
}

pub(super) fn live_login_lifecycle(
    result: Result<ProviderAuthLifecycle, crate::provider_launch_budget::ProviderLaunchError>,
    deadline_reached: bool,
) -> Option<ProviderAuthLifecycle> {
    if matches!(result, Ok(ProviderAuthLifecycle::Authenticated)) {
        Some(ProviderAuthLifecycle::Authenticated)
    } else {
        deadline_reached.then_some(ProviderAuthLifecycle::TimedOut)
    }
}

fn reprobe(
    session: &mut ProviderAuthSession,
    lifecycle: ProviderAuthLifecycle,
    probe_timeout: Duration,
) {
    if let Ok(observed) = auth_probe(session, probe_timeout) {
        if observed != lifecycle {
            session.settle(observed);
        }
    }
}

pub(super) fn monitor_login(
    sessions: Arc<Mutex<Vec<ProviderAuthSession>>>,
    provider: ProviderAuthProvider,
    challenge_id: String,
    timeout: Duration,
    probe_timeout: Duration,
) {
    thread::spawn(move || {
        loop {
            thread::sleep(timeout.min(Duration::from_secs(2)));
            let mut sessions = recover(&sessions);
            let Some(session) = sessions.iter_mut().find(|session| {
                session.provider == provider
                    && session.challenge_id() == Some(challenge_id.as_str())
                    && (session.login.is_some() || session.device_login.is_some())
            }) else {
                break;
            };
            refresh(session, probe_timeout);
        }
    });
}

fn auth_probe(
    session: &ProviderAuthSession,
    timeout: Duration,
) -> Result<ProviderAuthLifecycle, crate::provider_launch_budget::ProviderLaunchError> {
    let Some(control) = &session.lifecycle_control else {
        return Err(crate::provider_launch_budget::ProviderLaunchError::Failed(
            "provider authentication admission is unavailable".into(),
        ));
    };
    authentication(session.provider, &session.lock, timeout, control)
}
