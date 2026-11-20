use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger, LedgerError,
    NormalizedSessionBatchLedger, PublicProviderResolver, PublicProviderRunner,
    RunLifecycleFactLedger, TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::{DurableTurnPhase, HostEpoch};

use crate::public_driver_runtime::PublicDriversRuntime;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProviderSession {
    Running,
    Idle,
}

impl ProviderSession {
    pub(crate) const fn from_idle(idle: bool) -> Self {
        if idle { Self::Idle } else { Self::Running }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RunLiveness {
    pub(crate) turn_open: bool,
    pub(crate) session_running: bool,
    pub(crate) steers_pending: bool,
    pub(crate) releasing: bool,
}

impl RunLiveness {
    pub(crate) const fn idle(self) -> bool {
        !(self.turn_open || self.session_running || self.steers_pending || self.releasing)
    }

    pub(crate) const fn accepts_prompt(self) -> bool {
        !(self.turn_open || self.steers_pending || self.releasing)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RequestPlacement {
    RootTurn,
    OutsideRootTurn,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RunTurns {
    prompt_open: bool,
    provider_turn: Option<String>,
    session: ProviderSession,
    releasing: bool,
}

pub(crate) struct ProviderTurnScope<'a, L> {
    pub(crate) ledger: &'a L,
    pub(crate) conversation_id: &'a str,
    pub(crate) run_id: &'a str,
    pub(crate) host_epoch: HostEpoch,
}

pub(crate) struct TurnSettlement<'a, L, D, R> {
    pub(crate) runtime: &'a PublicDriversRuntime<L, D, R>,
    pub(crate) coordinator_id: &'a str,
    pub(crate) message_id: &'a str,
    pub(crate) host_epoch: HostEpoch,
    pub(crate) phase: DurableTurnPhase,
}

impl RunTurns {
    pub(crate) const fn prompt_started() -> Self {
        Self {
            prompt_open: true,
            provider_turn: None,
            session: ProviderSession::Idle,
            releasing: false,
        }
    }

    pub(crate) const fn settled() -> Self {
        Self {
            prompt_open: false,
            provider_turn: None,
            session: ProviderSession::Idle,
            releasing: false,
        }
    }

    pub(crate) fn open_prompt(&mut self) {
        self.prompt_open = true;
        self.releasing = false;
    }

    pub(crate) fn close_prompt(&mut self) -> bool {
        std::mem::replace(&mut self.prompt_open, false)
    }

    pub(crate) const fn prompt_open(&self) -> bool {
        self.prompt_open
    }

    pub(crate) const fn turn_open(&self) -> bool {
        self.prompt_open || self.provider_turn.is_some()
    }

    pub(crate) fn provider_turn(&self) -> Option<&str> {
        self.provider_turn.as_deref()
    }

    pub(crate) const fn releasing(&self) -> bool {
        self.releasing
    }

    pub(crate) fn begin_release(&mut self) -> bool {
        !std::mem::replace(&mut self.releasing, true)
    }

    pub(crate) fn report_session(&mut self, session: ProviderSession) -> bool {
        self.session = session;
        session == ProviderSession::Idle && self.provider_turn.is_some()
    }

    pub(crate) const fn liveness(&self, steers_pending: bool) -> RunLiveness {
        RunLiveness {
            turn_open: self.turn_open(),
            session_running: matches!(self.session, ProviderSession::Running),
            steers_pending,
            releasing: self.releasing,
        }
    }

    pub(crate) fn begin_provider_turn<L: NormalizedSessionBatchLedger>(
        &mut self,
        scope: &ProviderTurnScope<'_, L>,
    ) -> Result<Option<String>, LedgerError> {
        if self.provider_turn.is_some() {
            return Ok(None);
        }
        let turn_id = scope.ledger.start_provider_turn(
            scope.conversation_id,
            scope.run_id,
            scope.host_epoch,
        )?;
        self.provider_turn = Some(turn_id.clone());
        Ok(Some(turn_id))
    }

    pub(crate) const fn place_request(&self, from_background_child: bool) -> RequestPlacement {
        if self.turn_open() && !from_background_child {
            RequestPlacement::RootTurn
        } else {
            RequestPlacement::OutsideRootTurn
        }
    }

    pub(crate) fn settle_provider_turn<L: NormalizedSessionBatchLedger>(
        &mut self,
        ledger: &L,
        host_epoch: HostEpoch,
        phase: DurableTurnPhase,
    ) -> Result<bool, LedgerError> {
        let Some(turn_id) = self.provider_turn.take() else {
            return Ok(false);
        };
        ledger.settle_provider_turn(&turn_id, host_epoch, phase)?;
        Ok(true)
    }

    pub(crate) fn settle_prompt<L, D, R>(
        &mut self,
        settlement: &TurnSettlement<'_, L, D, R>,
    ) -> Result<bool, RuntimeError>
    where
        L: Clone
            + Ledger
            + RunLifecycleFactLedger
            + ConversationActivityLedger
            + TranscriptLedger
            + AgentChatPromptDispatchLedger,
        D: PublicProviderRunner + Clone,
        R: PublicProviderResolver,
    {
        if !self.prompt_open {
            return Ok(false);
        }
        settlement.runtime.settle_prompt_terminal(
            settlement.message_id,
            settlement.coordinator_id,
            settlement.host_epoch,
            settlement.phase,
        )?;
        Ok(self.close_prompt())
    }

    pub(crate) fn settle_open<L, D, R>(
        &mut self,
        settlement: &TurnSettlement<'_, L, D, R>,
    ) -> Result<bool, RuntimeError>
    where
        L: Clone
            + Ledger
            + RunLifecycleFactLedger
            + ConversationActivityLedger
            + TranscriptLedger
            + AgentChatPromptDispatchLedger
            + NormalizedSessionBatchLedger,
        D: PublicProviderRunner + Clone,
        R: PublicProviderResolver,
    {
        let provider_turn = self.settle_provider_turn(
            &settlement.runtime.ledger(),
            settlement.host_epoch,
            settlement.phase,
        )?;
        Ok(self.settle_prompt(settlement)? || provider_turn)
    }
}

#[cfg(test)]
#[path = "provider_run_state_tests.rs"]
mod tests;
