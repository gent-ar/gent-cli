use std::time::Instant;

use gent_drivers::claude_runner::ClaudeRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, NormalizedSessionBatchLedger, PendingPermissionLedger,
    PolicyLedger, PublicProviderResolver, TranscriptLedger,
};
use gent_protocol::{ProviderWarmOutcome, PublicRunOutcome};
use gent_runtime::RuntimeError;
use gent_types::{AgentChatPromptSaved, HostEpoch};

use super::launch::{LaunchKey, launch_key};
use super::{Binding, ClaudePromptExecution, ClaudePromptLifecycle, SessionRecovery};
use crate::provider_warm_pool::{WarmPoolRequest, WarmTarget, admission};
use crate::public_driver_runtime::run_failure::is_daemon_fatal;

const MAX_HELD_FACTS: usize = 64;

#[derive(Debug)]
pub(super) struct WarmBinding {
    pub(super) conversation_id: String,
    pub(super) last_dispatch: u64,
    pub(super) since: Instant,
    key: LaunchKey,
    held: Vec<PublicWireFact>,
}

impl<L, D, R> ClaudePromptLifecycle<L, D, R>
where
    L: Clone
        + Ledger
        + gent_ports::RunLifecycleFactLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + NormalizedSessionBatchLedger
        + AgentChatPromptDispatchLedger
        + gent_ports::AgentChatReadLedger
        + AgentChatRunContextReader
        + ConversationContentReader
        + gent_ports::AgentChatWorkspaceLedger
        + PendingPermissionLedger
        + PolicyLedger
        + gent_ports::AttachmentLedger
        + gent_ports::ToolSourceLedger
        + gent_ports::AgentChatConversationConfigLedger,
    D: ClaudePromptExecution + Clone,
    R: PublicProviderResolver,
{
    pub(crate) fn tend_warm_pool(
        &mut self,
        request: &WarmPoolRequest,
        host_epoch: HostEpoch,
    ) -> Result<Option<ProviderWarmOutcome>, RuntimeError> {
        self.reap(request.now)?;
        request
            .target
            .as_ref()
            .map(|target| self.warm_up(target, request.now, host_epoch))
            .transpose()
    }

    fn warm_up(
        &mut self,
        target: &WarmTarget,
        now: Instant,
        host_epoch: HostEpoch,
    ) -> Result<ProviderWarmOutcome, RuntimeError> {
        let (conversation_id, run_id) = (&target.conversation_id, &target.run_id);
        if !self.is_current(conversation_id, run_id) {
            return Ok(ProviderWarmOutcome::Superseded);
        }
        if let Some(warm) = self.warm.get_mut(run_id) {
            warm.since = now;
            return Ok(ProviderWarmOutcome::AlreadyWarm);
        }
        let evicted = match admission(
            self.pool_runs(),
            run_id,
            conversation_id,
            self.warm_capacity,
        ) {
            Ok(evicted) => evicted,
            Err(outcome) => {
                if let Some(binding) = self.active.get_mut(run_id) {
                    binding.idle_since = binding.idle_since.map(|_| now);
                }
                return Ok(outcome);
            }
        };
        let key = match launch_key(&self.runtime, conversation_id, run_id, &[]) {
            Ok(key) => key,
            Err(error) => return unavailable(run_id, error),
        };
        for victim in evicted {
            self.release_owned(&victim)?;
        }
        if let Err(error) = self
            .runner
            .prewarm_claude_prompt(run_id.clone(), key.0.clone())
        {
            return unavailable(run_id, error.into());
        }
        let started = self.runtime.runs().start_or_resume(super::start::request(
            run_id,
            &self.coordinator_id,
            host_epoch,
        ));
        match started.map(|response| response.outcome) {
            Ok(PublicRunOutcome::Started | PublicRunOutcome::Resumed) => {
                self.dispatches = self.dispatches.saturating_add(1);
                self.warm.insert(
                    run_id.clone(),
                    WarmBinding {
                        conversation_id: conversation_id.clone(),
                        last_dispatch: self.dispatches,
                        since: now,
                        key,
                        held: Vec::new(),
                    },
                );
                Ok(ProviderWarmOutcome::Warming)
            }
            Ok(_) => {
                self.runner.cancel_claude_prompt(run_id);
                Ok(ProviderWarmOutcome::Unavailable)
            }
            Err(error) => {
                self.runner.cancel_claude_prompt(run_id);
                unavailable(run_id, error)
            }
        }
    }

    pub(super) fn adopt_warm(
        &mut self,
        prompt: &AgentChatPromptSaved,
    ) -> Result<Vec<PublicWireFact>, RuntimeError> {
        let run_id = &prompt.run_id.0;
        let Some(warm) = self.warm.remove(run_id) else {
            return Ok(Vec::new());
        };
        let key = launch_key(
            &self.runtime,
            &prompt.message.conversation_id,
            run_id,
            &prompt.tool_source_ids,
        );
        let reusable = key.as_ref().is_ok_and(|key| *key == warm.key)
            && self.runner.has_claude_session(run_id);
        if !reusable {
            self.release_owned(run_id)?;
            return key.map(|_| Vec::new());
        }
        self.active.insert(
            run_id.clone(),
            Binding {
                prompt: prompt.clone(),
                sequence: 0,
                settled: true,
                interrupt_requested: false,
                steers: Vec::new(),
                session_recovery: SessionRecovery::NotNeeded,
                failure_reported_turn: None,
                open_children: std::collections::BTreeMap::new(),
                last_dispatch: warm.last_dispatch,
                idle_since: None,
                awaits_first_input: true,
            },
        );
        Ok(warm.held)
    }

    pub(super) fn flush_held(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        held: Vec<PublicWireFact>,
    ) -> Result<(), RuntimeError> {
        if !self.active.contains_key(run_id) {
            return Ok(());
        }
        for fact in held {
            self.record_wire(run_id, host_epoch, &fact)?;
        }
        Ok(())
    }

    pub(super) fn poll_warm(&mut self) {
        for run_id in self.warm.keys().cloned().collect::<Vec<_>>() {
            let kept = match self.runner.poll_claude_prompt(&run_id) {
                Ok(None) => true,
                Ok(Some(effects)) => self.hold(&run_id, effects),
                Err(_) => false,
            };
            if !kept {
                eprintln!("Claude warm process for run {run_id} stopped before a prompt");
                if self.runner.has_claude_session(&run_id) {
                    let _ = self.runner.release_claude_session(&run_id);
                }
                self.warm.remove(&run_id);
            }
        }
    }

    fn hold(&mut self, run_id: &str, effects: Vec<ClaudeRunnerEffect>) -> bool {
        let Some(warm) = self.warm.get_mut(run_id) else {
            return false;
        };
        for effect in effects {
            let ClaudeRunnerEffect::Fact(fact) = effect else {
                return false;
            };
            if matches!(fact, PublicWireFact::SessionStarted { .. })
                || warm.held.len() < MAX_HELD_FACTS
            {
                warm.held.push(fact);
            }
        }
        true
    }

    pub(super) fn is_current(&self, conversation_id: &str, run_id: &str) -> bool {
        self.runtime
            .ledger()
            .read_agent_chat_detail(conversation_id)
            .is_ok_and(|detail| detail.current_run_id == run_id)
    }
}

fn unavailable(run_id: &str, error: RuntimeError) -> Result<ProviderWarmOutcome, RuntimeError> {
    if is_daemon_fatal(&error) {
        return Err(error);
    }
    eprintln!("Claude warm-up for run {run_id} was unavailable: {error}");
    Ok(ProviderWarmOutcome::Unavailable)
}
