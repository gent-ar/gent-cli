use std::{
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use gent_core::{ProviderAuthEvent, ProviderAuthState, reduce_provider_auth};
use gent_drivers::{
    interrupt::{ProcessTreeControl, ProcessTreeSignal},
    process::SystemProcess,
    supervisor::ProviderProcess,
};
use gent_types::{ProviderAuthLifecycle, ProviderAuthProvider, ProviderAuthStatus, RunVersionLock};

use super::provider_auth_process::{authentication, now, public_lock};
use super::recover;

pub(super) struct ProviderAuthSession {
    pub(super) provider: ProviderAuthProvider,
    pub(super) state: ProviderAuthState,
    pub(super) lock: RunVersionLock,
    pub(super) login: Option<LoginProcess>,
}

pub(super) struct LoginProcess {
    pub(super) process: SystemProcess,
    pub(super) deadline: Instant,
}

impl ProviderAuthSession {
    pub(super) fn new(
        provider: ProviderAuthProvider,
        lock: RunVersionLock,
        lifecycle: ProviderAuthLifecycle,
    ) -> Self {
        let status = ProviderAuthStatus {
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
        }
    }

    pub(super) fn challenge_id(&self) -> Option<&str> {
        self.state
            .challenge
            .as_ref()
            .map(|value| value.challenge_id.as_str())
    }

    pub(super) fn status(&self) -> ProviderAuthStatus {
        self.state.status.clone().unwrap_or(ProviderAuthStatus {
            provider: self.provider,
            binary_lock: Some(public_lock(self.provider, &self.lock)),
            lifecycle: ProviderAuthLifecycle::Unauthenticated,
            selected_method: None,
            expires_at_unix_seconds: None,
        })
    }

    pub(super) fn settle(&mut self, lifecycle: ProviderAuthLifecycle) {
        if let Some(login) = self.login.take() {
            let _ = login.process.signal_tree(ProcessTreeSignal::Kill);
        }
        let mut status = self.status();
        status.lifecycle = lifecycle;
        self.state.status = Some(status);
    }
}

pub(super) fn refresh(session: &mut ProviderAuthSession, probe_timeout: Duration) {
    if let Some(login) = session.login.as_ref() {
        match login.process.try_exit_code() {
            Ok(Some(Some(0))) => {
                match authentication(session.provider, &session.lock, probe_timeout) {
                    Ok(ProviderAuthLifecycle::Authenticated) => {
                        session.login = None;
                        session.settle(ProviderAuthLifecycle::Authenticated);
                    }
                    Ok(_) => {
                        session.login = None;
                        session.settle(ProviderAuthLifecycle::Failed);
                    }
                    Err(_) => {}
                }
            }
            Ok(Some(_)) | Err(_) => {
                session.login = None;
                session.settle(ProviderAuthLifecycle::Failed);
            }
            Ok(None) if Instant::now() >= login.deadline => {
                session.settle(ProviderAuthLifecycle::TimedOut);
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

fn reprobe(
    session: &mut ProviderAuthSession,
    lifecycle: ProviderAuthLifecycle,
    probe_timeout: Duration,
) {
    if let Ok(observed) = authentication(session.provider, &session.lock, probe_timeout) {
        if observed != lifecycle {
            session.settle(observed);
        }
    }
}

pub(super) fn expire_login_after(
    sessions: Arc<Mutex<Vec<ProviderAuthSession>>>,
    provider: ProviderAuthProvider,
    timeout: Duration,
    probe_timeout: Duration,
) {
    thread::spawn(move || {
        thread::sleep(timeout);
        if let Some(session) = recover(&sessions)
            .iter_mut()
            .find(|session| session.provider == provider && session.login.is_some())
        {
            refresh(session, probe_timeout);
        }
    });
}
