use std::collections::BTreeSet;
use std::sync::{Mutex, MutexGuard};

use gent_core::{
    GoalPursuitStep, accounted_goal, blocked_admission, continuation_prompt,
    continuation_request_id, next_pursuit_step,
};
use gent_ports::{
    AgentChatPromptLedger, GoalLedger, GoalWrite, LedgerError, MAX_GOAL_TURN_OBSERVATIONS,
};
use gent_types::{
    AgentChatConversationId, AgentChatPromptCreate, AgentChatPromptDisposition,
    AgentChatPromptOrigin, AgentChatRequestId, AgentChatRunId, GoalRecord, HostEpoch, ReceiptId,
};

use crate::RuntimeError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoalContinuationWake {
    pub conversation_id: AgentChatConversationId,
    pub run_id: AgentChatRunId,
    pub receipt_id: ReceiptId,
    pub message_id: String,
}

pub trait GoalContinuationAdmission {
    fn admit(&mut self, wake: &GoalContinuationWake) -> Result<(), String>;
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GoalPursuitTick {
    pub admitted: Vec<GoalContinuationWake>,
    pub settled: Vec<GoalRecord>,
    pub failures: Vec<String>,
}

#[derive(Debug)]
pub struct GoalPursuitService<L> {
    ledger: L,
    woken: Mutex<BTreeSet<String>>,
}

impl<L> GoalPursuitService<L> {
    #[must_use]
    pub fn new(ledger: L) -> Self {
        Self {
            ledger,
            woken: Mutex::new(BTreeSet::new()),
        }
    }
}

impl<L: GoalLedger + AgentChatPromptLedger> GoalPursuitService<L> {
    pub fn tick(
        &self,
        host_epoch: HostEpoch,
        now: u64,
        admission: &mut dyn GoalContinuationAdmission,
    ) -> Result<GoalPursuitTick, RuntimeError> {
        let mut tick = GoalPursuitTick::default();
        for goal in self.ledger.active_goals()? {
            let pursuit = Pursuit {
                goal,
                host_epoch,
                now,
            };
            match self.pursue(&pursuit, admission, &mut tick) {
                Err(error @ RuntimeError::Ledger(LedgerError::IngressClosed { .. })) => {
                    return Err(error);
                }
                Err(error) => tick.failures.push(error.to_string()),
                Ok(()) => {}
            }
        }
        Ok(tick)
    }

    fn pursue(
        &self,
        pursuit: &Pursuit,
        admission: &mut dyn GoalContinuationAdmission,
        tick: &mut GoalPursuitTick,
    ) -> Result<(), RuntimeError> {
        let goal = &pursuit.goal;
        let turns = self.ledger.goal_turns(
            &goal.binding.conversation_id.0,
            goal.accounted_through_ordinal,
        )?;
        let accounted = accounted_goal(goal, &turns, pursuit.now);
        let goal = if accounted == *goal {
            accounted
        } else {
            match self
                .ledger
                .replace_goal(goal, &accounted, pursuit.host_epoch)?
            {
                GoalWrite::Updated(goal) => goal,
                GoalWrite::Current(_) => return Ok(()),
            }
        };
        if turns.len() == MAX_GOAL_TURN_OBSERVATIONS
            && turns.iter().all(|turn| turn.phase.is_terminal())
        {
            return Ok(());
        }
        let remaining = turns
            .into_iter()
            .filter(|turn| turn.ordinal > goal.accounted_through_ordinal)
            .collect::<Vec<_>>();
        match next_pursuit_step(&goal, &remaining, pursuit.now) {
            GoalPursuitStep::Idle | GoalPursuitStep::Wait => Ok(()),
            GoalPursuitStep::Settle(next) => self.settle(&goal, &next, pursuit, tick),
            GoalPursuitStep::Rewake { message_id } => {
                let Some(turn) = remaining.iter().find(|turn| turn.message_id == message_id) else {
                    return Ok(());
                };
                let wake = GoalContinuationWake {
                    conversation_id: goal.binding.conversation_id.clone(),
                    run_id: AgentChatRunId(turn.run_id.clone()),
                    receipt_id: ReceiptId(turn.receipt_id.clone()),
                    message_id,
                };
                if self.woken()?.contains(&wake.message_id) {
                    return Ok(());
                }
                self.wake(&goal, wake, pursuit, admission, tick)
            }
            GoalPursuitStep::Admit => self.admit(&goal, pursuit, admission, tick),
        }
    }

    fn admit(
        &self,
        goal: &GoalRecord,
        pursuit: &Pursuit,
        admission: &mut dyn GoalContinuationAdmission,
        tick: &mut GoalPursuitTick,
    ) -> Result<(), RuntimeError> {
        let request_id = continuation_request_id(goal);
        let origin = AgentChatPromptOrigin::GoalContinuation {
            goal_id: goal.binding.goal_id.clone(),
            continues_after_ordinal: goal.accounted_through_ordinal,
        };
        let create = AgentChatPromptCreate {
            request_id: AgentChatRequestId(request_id.clone()),
            receipt_id: ReceiptId(request_id),
            host_epoch: pursuit.host_epoch,
            conversation_id: goal.binding.conversation_id.clone(),
            disposition: AgentChatPromptDisposition::Send,
            text: continuation_prompt(goal),
            attachment_ids: Vec::new(),
            tool_source_ids: Vec::new(),
        };
        let saved = match self
            .ledger
            .save_agent_chat_prompt_with_origin(&create, &origin)
        {
            Ok(saved) => saved,
            Err(error @ LedgerError::IngressClosed { .. }) => return Err(error.into()),
            Err(error) => {
                tick.failures.push(error.to_string());
                return self.block(goal, pursuit, tick);
            }
        };
        let wake = GoalContinuationWake {
            conversation_id: goal.binding.conversation_id.clone(),
            run_id: saved.run_id,
            receipt_id: saved.receipt.receipt_id,
            message_id: saved.message.message_id,
        };
        self.wake(goal, wake, pursuit, admission, tick)
    }

    fn wake(
        &self,
        goal: &GoalRecord,
        wake: GoalContinuationWake,
        pursuit: &Pursuit,
        admission: &mut dyn GoalContinuationAdmission,
        tick: &mut GoalPursuitTick,
    ) -> Result<(), RuntimeError> {
        self.woken()?.insert(wake.message_id.clone());
        match admission.admit(&wake) {
            Ok(()) => {
                tick.admitted.push(wake);
                Ok(())
            }
            Err(reason) => {
                tick.failures.push(reason);
                self.block(goal, pursuit, tick)
            }
        }
    }

    fn block(
        &self,
        goal: &GoalRecord,
        pursuit: &Pursuit,
        tick: &mut GoalPursuitTick,
    ) -> Result<(), RuntimeError> {
        blocked_admission(goal, pursuit.now)
            .map_or(Ok(()), |next| self.settle(goal, &next, pursuit, tick))
    }

    fn settle(
        &self,
        goal: &GoalRecord,
        next: &GoalRecord,
        pursuit: &Pursuit,
        tick: &mut GoalPursuitTick,
    ) -> Result<(), RuntimeError> {
        if let GoalWrite::Updated(goal) =
            self.ledger.replace_goal(goal, next, pursuit.host_epoch)?
        {
            tick.settled.push(goal);
        }
        Ok(())
    }

    fn woken(&self) -> Result<MutexGuard<'_, BTreeSet<String>>, RuntimeError> {
        self.woken.lock().map_err(|_| {
            RuntimeError::Ledger(LedgerError::Invariant(
                "goal pursuit wake registry is unavailable".into(),
            ))
        })
    }
}

struct Pursuit {
    goal: GoalRecord,
    host_epoch: HostEpoch,
    now: u64,
}
