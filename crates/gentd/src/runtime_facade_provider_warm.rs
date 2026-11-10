use gent_ports::ConversationLedger;
use gent_protocol::{AgentChatIntentFrame, AgentChatProviderWarm, ProviderWarmOutcome};
use gent_types::{AgentChatProvider, ProviderAuthProvider};

use super::RuntimeFacade;
use crate::agent_chat_intent_error::AgentChatIntentError;
use crate::provider_auth_api::{LaunchAdmission, ProviderAuthPort, launch_admission};
use crate::provider_warm_pool::{WarmPoolRequest, WarmTarget};

impl RuntimeFacade {
    pub(super) fn provider_warm_intent(
        &self,
        frame: &AgentChatIntentFrame,
    ) -> Result<Option<AgentChatIntentFrame>, AgentChatIntentError> {
        let AgentChatIntentFrame::WarmProvider(warm) = frame else {
            return Ok(None);
        };
        Ok(Some(AgentChatIntentFrame::ProviderWarm(
            warm.result(self.warm_provider(warm)?),
        )))
    }

    fn warm_provider(
        &self,
        warm: &AgentChatProviderWarm,
    ) -> Result<ProviderWarmOutcome, AgentChatIntentError> {
        let Some(ingress) = &self.ordinary_prompt_ingress else {
            return Ok(ProviderWarmOutcome::Unavailable);
        };
        let provider = self
            .agent_chat_reads
            .as_ref()
            .ok_or("agent-chat reads are unavailable")?
            .run_selection(&warm.conversation_id.0, &warm.run_id.0)?
            .provider;
        if !launch_admitted(self.provider_auth.as_deref(), provider) {
            return Ok(ProviderWarmOutcome::Unavailable);
        }
        let active_turn = self
            .transcript_import_ledger
            .list_run_turns(&warm.run_id.0)
            .map_err(|error| error.to_string())?
            .iter()
            .any(|turn| !turn.phase.is_terminal());
        if active_turn {
            return Ok(ProviderWarmOutcome::TurnActive);
        }
        Ok(ingress
            .tend_warm_pool(Some(provider), &warm_request(warm))?
            .unwrap_or(ProviderWarmOutcome::Unavailable))
    }

    pub(super) fn reap_superseded_warm_processes(&self) {
        let Some(ingress) = &self.ordinary_prompt_ingress else {
            return;
        };
        let request = WarmPoolRequest {
            now: std::time::Instant::now(),
            target: None,
        };
        if let Err(error) = ingress.tend_warm_pool(None, &request) {
            eprintln!("superseded warm processes were not stopped: {error}");
        }
    }
}

fn launch_admitted(auth: Option<&dyn ProviderAuthPort>, provider: AgentChatProvider) -> bool {
    let account = match provider {
        AgentChatProvider::Claude => ProviderAuthProvider::Claude,
        AgentChatProvider::Codex => ProviderAuthProvider::Codex,
        AgentChatProvider::Claurst => return true,
    };
    auth.is_none_or(|auth| launch_admission(auth, account) == LaunchAdmission::Admitted)
}

fn warm_request(warm: &AgentChatProviderWarm) -> WarmPoolRequest {
    WarmPoolRequest {
        now: std::time::Instant::now(),
        target: Some(WarmTarget {
            conversation_id: warm.conversation_id.0.clone(),
            run_id: warm.run_id.0.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use gent_types::{AgentChatProvider, ProviderAuthLifecycle};

    use super::launch_admitted;
    use crate::provider_auth_api::fixed::FixedProviderAuth;

    #[test]
    fn only_a_signed_in_public_provider_is_warmed() {
        for (lifecycle, admitted) in [
            (ProviderAuthLifecycle::Authenticated, true),
            (ProviderAuthLifecycle::Checking, false),
            (ProviderAuthLifecycle::Unauthenticated, false),
            (ProviderAuthLifecycle::NotInstalled, false),
            (ProviderAuthLifecycle::Expired, false),
        ] {
            let auth = FixedProviderAuth(lifecycle);
            assert_eq!(
                launch_admitted(Some(&auth), AgentChatProvider::Claude),
                admitted
            );
            assert_eq!(
                launch_admitted(Some(&auth), AgentChatProvider::Codex),
                admitted
            );
            assert!(launch_admitted(Some(&auth), AgentChatProvider::Claurst));
        }
        assert!(launch_admitted(None, AgentChatProvider::Claude));
    }
}
