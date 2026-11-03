use super::provider_auth_codex::CodexDeviceLogin;
use super::provider_auth_session::{monitor_login, refresh};
use super::{StandaloneProviderAuthPort, recover};
use gent_drivers::lock::rechecked_identity;
use gent_protocol::ProviderAuthFrame;
use gent_types::{
    ProviderAuthHandoffMethod as Method, ProviderAuthHandoffState, ProviderAuthLifecycle,
    ProviderAuthMethod, ProviderAuthMethodSelection, ProviderAuthProvider,
};
use std::sync::Arc;

impl StandaloneProviderAuthPort {
    pub(super) fn handoff_state(
        &self,
        request_id: String,
        provider: ProviderAuthProvider,
    ) -> ProviderAuthFrame {
        let sessions = recover(&self.sessions);
        let session = sessions.iter().find(|item| item.provider == provider);
        let status = session.map_or_else(
            || super::provider_auth_check::unlocked(provider, ProviderAuthLifecycle::Checking),
            |session| session.status(),
        );
        let challenge = session.and_then(|session| session.state.challenge.clone());
        let mut methods = vec![Method::AccountBrowser];
        if session.is_some_and(|session| device_supported(provider, &session.lock.version)) {
            methods.push(Method::DeviceVerification);
        }
        let handoff = session
            .and_then(|session| session.device_login.as_ref())
            .and_then(CodexDeviceLogin::handoff);
        ProviderAuthFrame::HandoffState {
            request_id,
            state: ProviderAuthHandoffState {
                status,
                challenge,
                methods,
                handoff,
            },
        }
    }

    pub(super) fn select_handoff(
        &self,
        request_id: String,
        challenge_id: String,
        method: Method,
    ) -> Result<ProviderAuthFrame, String> {
        if method == Method::AccountBrowser {
            let reply = self.select(
                request_id.clone(),
                ProviderAuthMethodSelection {
                    challenge_id,
                    method: ProviderAuthMethod::AccountBrowser,
                },
            )?;
            let ProviderAuthFrame::SelectionAccepted { status, .. } = reply else {
                return Err("provider authentication selection failed".into());
            };
            return Ok(self.handoff_state(request_id, status.provider));
        }
        let provider;
        {
            let mut sessions = recover(&self.sessions);
            let session = sessions
                .iter_mut()
                .find(|session| session.challenge_id() == Some(challenge_id.as_str()))
                .ok_or_else(|| {
                    "provider authentication challenge is no longer active".to_owned()
                })?;
            provider = session.provider;
            refresh(session, self.probe_timeout);
            let challenge = session.state.challenge.as_ref().ok_or_else(|| {
                "provider authentication challenge is no longer active".to_owned()
            })?;
            if !device_supported(provider, &session.lock.version) || session.login.is_some() {
                return Err("provider authentication handoff is unavailable".into());
            }
            if session.device_login.is_none() {
                if challenge.expires_at_unix_seconds <= super::provider_auth_process::now() {
                    session.settle(ProviderAuthLifecycle::Expired);
                } else if rechecked_identity(&session.lock).ok().as_ref() != Some(&session.lock) {
                    session.settle(ProviderAuthLifecycle::ProviderChanged);
                } else {
                    let login = CodexDeviceLogin::start(
                        &session.lock,
                        challenge.expires_at_unix_seconds,
                        self.control()?,
                    )?;
                    let mut status = session.status();
                    status.lifecycle = ProviderAuthLifecycle::Verifying;
                    status.selected_method = None;
                    session.state.status = Some(status);
                    session.device_login = Some(login);
                    monitor_login(
                        Arc::clone(&self.sessions),
                        provider,
                        challenge_id,
                        self.login_timeout,
                        self.probe_timeout,
                    );
                }
            }
        }
        Ok(self.handoff_state(request_id, provider))
    }
}

fn device_supported(provider: ProviderAuthProvider, version: &str) -> bool {
    if provider != ProviderAuthProvider::Codex {
        return false;
    }
    let Some(version) = version.split_whitespace().last() else {
        return false;
    };
    let mut components = version.split('.');
    matches!((components.next().and_then(|n| n.parse::<u32>().ok()), components.next().and_then(|n| n.parse::<u32>().ok())), (Some(major), Some(minor)) if major > 0 || minor >= 145)
}
