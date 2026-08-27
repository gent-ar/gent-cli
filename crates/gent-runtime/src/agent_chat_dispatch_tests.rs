use std::sync::{Arc, Mutex};

use gent_ports::{AgentChatPromptDispatchLedger, LedgerError};
use gent_types::{AgentChatPromptSaved, AgentChatProvider, HostEpoch};

use super::{
    AgentChatPromptDispatchAuthority, AgentChatPromptDispatchResult, AgentChatPromptDispatchService,
};

#[derive(Clone, Default)]
struct Ledger(Arc<Mutex<u8>>);

impl AgentChatPromptDispatchLedger for Ledger {
    fn claim_agent_chat_prompt_dispatch(
        &self,
        _: &str,
        _: HostEpoch,
        _: AgentChatProvider,
    ) -> Result<Option<AgentChatPromptSaved>, LedgerError> {
        *self.0.lock().unwrap() += 1;
        Ok(None)
    }

    fn begin_agent_chat_prompt_launch(
        &self,
        _: &str,
        _: &str,
        _: HostEpoch,
    ) -> Result<(), LedgerError> {
        unreachable!("observer test must not launch")
    }

    fn confirm_agent_chat_prompt_started(
        &self,
        _: &str,
        _: &str,
        _: HostEpoch,
    ) -> Result<(), LedgerError> {
        unreachable!("observer test must not confirm")
    }

    fn release_agent_chat_prompt_claim(
        &self,
        _: &str,
        _: &str,
        _: HostEpoch,
    ) -> Result<(), LedgerError> {
        unreachable!("observer test must not release")
    }

    fn release_agent_chat_prompt_unstarted_launch(
        &self,
        _: &str,
        _: &str,
        _: HostEpoch,
    ) -> Result<(), LedgerError> {
        unreachable!("observer test must not release")
    }

    fn mark_agent_chat_prompt_unprovable(
        &self,
        _: &str,
        _: &str,
        _: HostEpoch,
    ) -> Result<(), LedgerError> {
        unreachable!("observer test must not mark")
    }

    fn settle_agent_chat_prompt_dispatch(
        &self,
        _: &str,
        _: &str,
        _: HostEpoch,
    ) -> Result<(), LedgerError> {
        unreachable!("observer test must not settle")
    }

    fn recover_agent_chat_prompt_dispatches(&self, _: HostEpoch) -> Result<(), LedgerError> {
        unreachable!("observer test must not recover")
    }
}

#[test]
fn observer_does_not_read_or_claim_provider_bound_prompts() {
    let ledger = Ledger::default();
    let service = AgentChatPromptDispatchService::new(
        ledger.clone(),
        AgentChatPromptDispatchAuthority::Observer,
    );
    assert_eq!(
        service
            .claim("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
            .unwrap(),
        AgentChatPromptDispatchResult::DeniedObserver
    );
    assert_eq!(*ledger.0.lock().unwrap(), 0);
}

#[test]
fn approved_service_exposes_only_an_exclusive_durable_claim() {
    let ledger = Ledger::default();
    let service = AgentChatPromptDispatchService::new(
        ledger.clone(),
        AgentChatPromptDispatchAuthority::Approved,
    );
    assert_eq!(
        service
            .claim("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
            .unwrap(),
        AgentChatPromptDispatchResult::Empty
    );
    assert_eq!(*ledger.0.lock().unwrap(), 1);
}
