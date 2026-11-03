use std::{sync::Arc, time::Instant};

use gent_core::{ProviderAuthEffect, ProviderAuthEvent, reduce_provider_auth};
use gent_drivers::lock::rechecked_identity;
use gent_protocol::ProviderAuthFrame;
use gent_types::ProviderAuthLifecycle;

use super::provider_auth_process::{launch, login_arguments, now};
use super::provider_auth_session::{LoginProcess, monitor_login};
use super::{StandaloneProviderAuthPort, recover};

impl StandaloneProviderAuthPort {
    pub(super) fn select(
        &self,
        request_id: String,
        selection: gent_types::ProviderAuthMethodSelection,
    ) -> Result<ProviderAuthFrame, String> {
        let mut sessions = recover(&self.sessions);
        let session = sessions
            .iter_mut()
            .find(|item| item.challenge_id() == Some(selection.challenge_id.as_str()))
            .ok_or_else(|| "provider authentication challenge is no longer active".to_owned())?;
        if session.device_login.is_some() {
            return Err(
                "provider sign-in is already active; cancel it before changing methods".into(),
            );
        }
        let (state, effect) = reduce_provider_auth(
            session.state.clone(),
            ProviderAuthEvent::SelectMethod {
                selection,
                now: now(),
            },
        );
        session.state = state;
        match effect {
            ProviderAuthEffect::BeginLogin { .. }
                if rechecked_identity(&session.lock).ok().as_ref() != Some(&session.lock) =>
            {
                session.settle(ProviderAuthLifecycle::ProviderChanged);
            }
            ProviderAuthEffect::BeginLogin { .. } => match launch(
                session.provider,
                &session.lock,
                login_arguments(session.provider),
                self.control()?,
            ) {
                Ok(process) => {
                    session.login = Some(LoginProcess {
                        process,
                        deadline: Instant::now() + self.login_timeout,
                        next_auth_check: Instant::now(),
                    });
                    monitor_login(
                        Arc::clone(&self.sessions),
                        session.provider,
                        session.challenge_id().unwrap_or_default().into(),
                        self.login_timeout,
                        self.probe_timeout,
                    );
                }
                Err(_) => session.settle(ProviderAuthLifecycle::Failed),
            },
            ProviderAuthEffect::None => {}
            ProviderAuthEffect::Status(status) => session.settle(status.lifecycle),
            ProviderAuthEffect::Rejected(_) | ProviderAuthEffect::AskTool(_) => {
                return Err("provider authentication method was rejected".into());
            }
        }
        Ok(ProviderAuthFrame::SelectionAccepted {
            request_id,
            status: session.status(),
        })
    }

    pub(super) fn cancel(
        &self,
        request_id: String,
        challenge_id: String,
    ) -> Result<ProviderAuthFrame, String> {
        let mut sessions = recover(&self.sessions);
        let session = sessions
            .iter_mut()
            .find(|item| item.challenge_id() == Some(challenge_id.as_str()))
            .ok_or_else(|| "provider authentication challenge is no longer active".to_owned())?;
        if session.device_login.is_some() {
            super::refresh(session, self.probe_timeout);
            if session.challenge_id().is_none() {
                return Ok(ProviderAuthFrame::Status {
                    request_id,
                    status: session.status(),
                });
            }
        }
        let (state, effect) = reduce_provider_auth(
            session.state.clone(),
            ProviderAuthEvent::Cancel { challenge_id },
        );
        session.state = state;
        if let ProviderAuthEffect::Status(status) = effect {
            session.settle(status.lifecycle);
            session.state.status = Some(status);
        }
        Ok(ProviderAuthFrame::Status {
            request_id,
            status: session.status(),
        })
    }
}
