use gent_protocol::ProviderAuthFrame;
use gent_types::{ProviderAuthLifecycle, ProviderAuthProvider};

use super::provider_auth_check::unlocked;
use super::provider_auth_process::{
    authentication, exit_code_bounded, identified_lock, launch, logout_arguments,
};
use super::provider_auth_session::ProviderAuthSession;
use super::{StandaloneProviderAuthPort, recover};

impl StandaloneProviderAuthPort {
    pub(super) fn logout(
        &self,
        request_id: String,
        provider: ProviderAuthProvider,
    ) -> Result<ProviderAuthFrame, String> {
        let Some(executable) = self.existing_executable(provider) else {
            return Ok(ProviderAuthFrame::Status {
                request_id,
                status: unlocked(provider, ProviderAuthLifecycle::NotInstalled),
            });
        };
        let control = self.control()?;
        let lock = identified_lock(provider, &executable, self.probe_timeout, control)
            .map_err(|_| "provider sign-out could not identify the provider".to_owned())?;
        let mut sessions = recover(&self.sessions);
        if let Some(index) = sessions.iter().position(|item| item.provider == provider) {
            sessions
                .remove(index)
                .settle(ProviderAuthLifecycle::Cancelled);
        }
        let process = launch(provider, &lock, logout_arguments(provider), control)?;
        exit_code_bounded(&process, self.probe_timeout)
            .map_err(|_| "provider sign-out did not finish".to_owned())?;
        let lifecycle = authentication(provider, &lock, self.probe_timeout, control)
            .map_err(|_| "provider sign-out could not be verified".to_owned())?;
        let session = ProviderAuthSession::new(provider, lock, lifecycle, Some(control.clone()));
        let status = session.status();
        sessions.push(session);
        Ok(ProviderAuthFrame::Status { request_id, status })
    }
}
