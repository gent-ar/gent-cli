use std::{
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use gent_core::{ProviderAuthEffect, ProviderAuthEvent, ProviderAuthState, reduce_provider_auth};
use gent_protocol::ProviderAuthFrame;
use gent_types::{
    ProviderAuthChallenge, ProviderAuthLifecycle, ProviderAuthMethod, ProviderAuthProvider,
    ProviderAuthStatus,
};

use self::provider_auth_check::{ProviderAuthChecks, unlocked};
use self::provider_auth_process::{
    PROBE_TIMEOUT, agent_provider, authentication, challenge_id, identified_lock, now, public_lock,
};
use self::provider_auth_session::{ProviderAuthSession, refresh};
use crate::provider_launch_budget::{ProbeRetry, ProviderLaunchError};

const AUTH_WINDOW: Duration = Duration::from_secs(600);
const STATUS_ANSWER_WINDOW: Duration = Duration::from_secs(5);
pub(crate) const STATUS_SETTLE_BOUND: Duration =
    ProbeRetry::BACKGROUND.settle_bound(PROBE_TIMEOUT.saturating_mul(2));

#[path = "provider_auth_process.rs"]
mod provider_auth_process;

#[path = "provider_auth_session.rs"]
mod provider_auth_session;

#[path = "provider_auth_challenge.rs"]
mod provider_auth_challenge;

#[path = "provider_auth_check.rs"]
mod provider_auth_check;

pub(crate) trait ProviderAuthPort: Send + Sync {
    fn exchange(&self, frame: ProviderAuthFrame) -> Result<ProviderAuthFrame, String>;
    fn check_status(&self, provider: ProviderAuthProvider) -> ProviderAuthStatus;
}

#[derive(Clone)]
pub(crate) struct StandaloneProviderAuthPort {
    executables: crate::provider_executables::ProviderExecutables,
    sessions: Arc<Mutex<Vec<ProviderAuthSession>>>,
    checks: ProviderAuthChecks,
    challenge_window: Duration,
    login_timeout: Duration,
    probe_timeout: Duration,
    probe_retry: ProbeRetry,
    answer_window: Duration,
}

impl StandaloneProviderAuthPort {
    pub(crate) fn new(executables: crate::provider_executables::ProviderExecutables) -> Self {
        Self {
            executables,
            sessions: Arc::new(Mutex::new(Vec::new())),
            checks: Arc::new(Mutex::new(Vec::new())),
            challenge_window: AUTH_WINDOW,
            login_timeout: AUTH_WINDOW,
            probe_timeout: PROBE_TIMEOUT,
            probe_retry: ProbeRetry::BACKGROUND,
            answer_window: STATUS_ANSWER_WINDOW,
        }
    }

    #[cfg(test)]
    fn with_windows(mut self, challenge_window: Duration, login_timeout: Duration) -> Self {
        self.challenge_window = challenge_window;
        self.login_timeout = login_timeout;
        self
    }

    #[cfg(test)]
    fn with_probe_budget(
        mut self,
        probe_timeout: Duration,
        probe_retry: ProbeRetry,
        answer_window: Duration,
    ) -> Self {
        self.probe_timeout = probe_timeout;
        self.probe_retry = probe_retry;
        self.answer_window = answer_window;
        self
    }

    fn login(
        &self,
        request_id: String,
        provider: ProviderAuthProvider,
    ) -> Result<ProviderAuthFrame, String> {
        let checking = |request_id| ProviderAuthFrame::Status {
            request_id,
            status: unlocked(provider, ProviderAuthLifecycle::Checking),
        };
        let Some(executable) = self.existing_executable(provider) else {
            return Ok(ProviderAuthFrame::Status {
                request_id,
                status: self.checked_status(provider),
            });
        };
        let lock = match identified_lock(provider, &executable, self.probe_timeout) {
            Ok(lock) => lock,
            Err(ProviderLaunchError::TimedOut(_)) => return Ok(checking(request_id)),
            Err(ProviderLaunchError::Failed(message)) => return Err(message),
        };
        let mut sessions = recover(&self.sessions);
        if let Some(index) = sessions.iter().position(|item| item.provider == provider) {
            refresh(&mut sessions[index], self.probe_timeout);
            if sessions[index].login.is_some()
                || sessions[index].status().lifecycle == ProviderAuthLifecycle::Authenticated
            {
                return Ok(ProviderAuthFrame::Status {
                    request_id,
                    status: sessions[index].status(),
                });
            }
            sessions.remove(index);
        }
        let Ok(lifecycle) = authentication(provider, &lock, self.probe_timeout) else {
            return Ok(checking(request_id));
        };
        match lifecycle {
            ProviderAuthLifecycle::Unauthenticated => {}
            ProviderAuthLifecycle::Authenticated => {
                let session =
                    ProviderAuthSession::new(provider, lock, ProviderAuthLifecycle::Authenticated);
                let status = session.status();
                sessions.push(session);
                return Ok(ProviderAuthFrame::Status { request_id, status });
            }
            lifecycle => {
                let status = ProviderAuthSession::new(provider, lock, lifecycle).status();
                return Ok(ProviderAuthFrame::Status { request_id, status });
            }
        }
        let challenge = ProviderAuthChallenge {
            challenge_id: challenge_id(provider, &request_id, &lock.digest_sha256),
            provider,
            binary_lock: public_lock(provider, &lock),
            methods: vec![ProviderAuthMethod::AccountBrowser],
            expires_at_unix_seconds: now().saturating_add(self.challenge_window.as_secs()),
        };
        let (state, effect) = reduce_provider_auth(
            ProviderAuthState::default(),
            ProviderAuthEvent::ObservedUnauthenticated { challenge },
        );
        sessions.push(ProviderAuthSession {
            provider,
            state,
            lock,
            login: None,
        });
        match effect {
            ProviderAuthEffect::AskTool(challenge) => Ok(ProviderAuthFrame::AskTool {
                request_id,
                challenge,
            }),
            _ => Err("provider authentication challenge could not be created".into()),
        }
    }

    fn existing_executable(&self, provider: ProviderAuthProvider) -> Option<PathBuf> {
        self.executables.executable(agent_provider(provider))
    }
}

impl ProviderAuthPort for StandaloneProviderAuthPort {
    fn exchange(&self, frame: ProviderAuthFrame) -> Result<ProviderAuthFrame, String> {
        match frame {
            ProviderAuthFrame::StatusRequest {
                request_id,
                provider,
            } => Ok(ProviderAuthFrame::Status {
                request_id,
                status: self.checked_status(provider),
            }),
            ProviderAuthFrame::LoginRequest {
                request_id,
                provider,
            } => self.login(request_id, provider),
            ProviderAuthFrame::SelectMethod {
                request_id,
                selection,
            } => self.select(request_id, selection),
            ProviderAuthFrame::Cancel {
                request_id,
                challenge_id,
            } => self.cancel(request_id, challenge_id),
            _ => Err("provider authentication frame is server-only".into()),
        }
    }

    fn check_status(&self, provider: ProviderAuthProvider) -> ProviderAuthStatus {
        self.checked_status(provider)
    }
}

fn recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
#[path = "provider_auth_api_tests.rs"]
mod tests;
