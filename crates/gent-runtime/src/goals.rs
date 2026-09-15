//! Observer-gated coordination of durable, provider-neutral user goals.

use gent_core::{
    GoalDraft, GoalRejection, GoalUserCommand, apply_user_command, create_goal, replaced_goal,
    reported_goal, stopped_goal,
};
use gent_ports::{
    ActiveGoalResolver, GoalLedger, GoalWrite, LedgerError, MAX_GOAL_TURN_OBSERVATIONS,
};
use gent_types::{
    AgentChatConversationId, GoalDispatchState, GoalProjection, GoalRecord, GoalReportOutcome,
    GoalStatus, HostEpoch,
};
use sha2::{Digest, Sha256};

use crate::RuntimeError;

const MAX_WRITE_ATTEMPTS: usize = 4;

/// Explicit composition authority for durable `/goal` operations.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GoalAuthority {
    /// Shipped observer behavior: no goal data is read or written.
    #[default]
    Observer,
    /// Reserved for an evidence-approved daemon composition root.
    Approved,
}

/// Result returned by an observer-gated durable goal operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GoalResult {
    DeniedObserver,
    Goal(Option<GoalRecord>),
    Rejected(GoalRejection),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GoalControl {
    Pause,
    Resume,
    Clear,
}

/// Coordinates pure goal transitions with revision-fenced durable storage.
#[derive(Clone, Debug)]
pub struct GoalService<L> {
    ledger: L,
    authority: GoalAuthority,
}

impl<L> GoalService<L> {
    /// Creates an inert goal service unless composition explicitly approves it.
    #[must_use]
    pub fn new(ledger: L, authority: GoalAuthority) -> Self {
        Self { ledger, authority }
    }
}

impl<L: GoalLedger> GoalService<L> {
    pub fn set(
        &self,
        request_id: &str,
        conversation_id: &AgentChatConversationId,
        objective: String,
        token_budget: Option<u64>,
        host_epoch: HostEpoch,
        now: u64,
    ) -> Result<GoalResult, RuntimeError> {
        if self.authority != GoalAuthority::Approved {
            return Ok(GoalResult::DeniedObserver);
        }
        let goal_id = goal_id_for(conversation_id, request_id);
        for _ in 0..MAX_WRITE_ATTEMPTS {
            let current = self.ledger.current_goal(&conversation_id.0)?;
            if let Some(current) = current
                .as_ref()
                .filter(|goal| goal.binding.goal_id == goal_id)
            {
                return Ok(GoalResult::Goal(Some(current.clone())));
            }
            let draft = GoalDraft {
                goal_id: goal_id.clone(),
                conversation_id: conversation_id.clone(),
                objective: objective.clone(),
                token_budget,
                accounted_through_ordinal: self.settled_prefix(&conversation_id.0, 0)?,
            };
            let goal = match create_goal(draft, now) {
                Ok(goal) => goal,
                Err(rejection) => return Ok(GoalResult::Rejected(rejection)),
            };
            let replaced = current
                .as_ref()
                .and_then(|current| replaced_goal(current, now));
            if let GoalWrite::Updated(goal) =
                self.ledger
                    .create_goal(current.as_ref(), replaced.as_ref(), &goal, host_epoch)?
            {
                return Ok(GoalResult::Goal(Some(goal)));
            }
        }
        Err(contention())
    }

    pub fn control(
        &self,
        conversation_id: &AgentChatConversationId,
        goal_id: &str,
        expected_revision: u64,
        control: GoalControl,
        host_epoch: HostEpoch,
        now: u64,
    ) -> Result<GoalResult, RuntimeError> {
        if self.authority != GoalAuthority::Approved {
            return Ok(GoalResult::DeniedObserver);
        }
        let Some(current) = self
            .ledger
            .current_goal(&conversation_id.0)?
            .filter(|goal| goal.binding.goal_id == goal_id)
        else {
            return Ok(GoalResult::Rejected(GoalRejection::Missing));
        };
        let command = match control {
            GoalControl::Pause => GoalUserCommand::Pause,
            GoalControl::Clear => GoalUserCommand::Clear,
            GoalControl::Resume => GoalUserCommand::Resume {
                accounted_through_ordinal: self
                    .settled_prefix(&conversation_id.0, current.accounted_through_ordinal)?,
            },
        };
        let next = match apply_user_command(&current, expected_revision, command, now) {
            Ok(next) => next,
            Err(rejection) => return Ok(GoalResult::Rejected(rejection)),
        };
        Ok(
            match self.ledger.replace_goal(&current, &next, host_epoch)? {
                GoalWrite::Updated(goal) => GoalResult::Goal(Some(goal)),
                GoalWrite::Current(_) => GoalResult::Rejected(GoalRejection::RevisionMismatch),
            },
        )
    }

    pub fn report(
        &self,
        goal_id: &str,
        outcome: GoalReportOutcome,
        note: Option<String>,
        host_epoch: HostEpoch,
        now: u64,
    ) -> Result<GoalResult, RuntimeError> {
        if self.authority != GoalAuthority::Approved {
            return Ok(GoalResult::DeniedObserver);
        }
        for _ in 0..MAX_WRITE_ATTEMPTS {
            let Some(current) = self.ledger.find_goal(goal_id)? else {
                return Ok(GoalResult::Rejected(GoalRejection::Missing));
            };
            if !self.turn_in_flight(&current)? {
                return Ok(GoalResult::Rejected(GoalRejection::NoActiveTurn));
            }
            let next = match reported_goal(&current, outcome, note.clone(), now) {
                Ok(next) => next,
                Err(rejection) => return Ok(GoalResult::Rejected(rejection)),
            };
            if let GoalWrite::Updated(goal) =
                self.ledger.replace_goal(&current, &next, host_epoch)?
            {
                return Ok(GoalResult::Goal(Some(goal)));
            }
        }
        Err(contention())
    }

    pub fn stop(
        &self,
        conversation_id: &AgentChatConversationId,
        host_epoch: HostEpoch,
        now: u64,
    ) -> Result<Option<GoalRecord>, RuntimeError> {
        if self.authority != GoalAuthority::Approved {
            return Ok(None);
        }
        for _ in 0..MAX_WRITE_ATTEMPTS {
            let Some(current) = self.ledger.current_goal(&conversation_id.0)? else {
                return Ok(None);
            };
            let Some(next) = stopped_goal(&current, now) else {
                return Ok(None);
            };
            if let GoalWrite::Updated(goal) =
                self.ledger.replace_goal(&current, &next, host_epoch)?
            {
                return Ok(Some(goal));
            }
        }
        Err(contention())
    }

    pub fn current(
        &self,
        conversation_id: &AgentChatConversationId,
    ) -> Result<GoalResult, RuntimeError> {
        if self.authority != GoalAuthority::Approved {
            return Ok(GoalResult::DeniedObserver);
        }
        Ok(GoalResult::Goal(
            self.ledger
                .current_goal(&conversation_id.0)?
                .filter(|goal| goal.status != GoalStatus::Cleared),
        ))
    }

    fn turn_in_flight(&self, goal: &GoalRecord) -> Result<bool, RuntimeError> {
        let mut after = goal.accounted_through_ordinal;
        loop {
            let turns = self
                .ledger
                .goal_turns(&goal.binding.conversation_id.0, after)?;
            if turns.iter().any(|turn| {
                !turn.phase.is_terminal() && turn.dispatch == GoalDispatchState::InFlight
            }) {
                return Ok(true);
            }
            match turns.last() {
                Some(last) if turns.len() == MAX_GOAL_TURN_OBSERVATIONS => after = last.ordinal,
                _ => return Ok(false),
            }
        }
    }

    fn settled_prefix(&self, conversation_id: &str, from: u64) -> Result<u64, RuntimeError> {
        let mut settled = from;
        loop {
            let turns = self.ledger.goal_turns(conversation_id, settled)?;
            for turn in &turns {
                if !turn.phase.is_terminal() {
                    return Ok(settled);
                }
                settled = turn.ordinal;
            }
            if turns.len() < MAX_GOAL_TURN_OBSERVATIONS {
                return Ok(settled);
            }
        }
    }
}

impl<L: GoalLedger + std::fmt::Debug> ActiveGoalResolver for GoalService<L> {
    fn resolve_active_goal(
        &self,
        conversation_id: &str,
    ) -> Result<Option<GoalProjection>, LedgerError> {
        if self.authority != GoalAuthority::Approved {
            return Ok(None);
        }
        self.ledger
            .current_goal(conversation_id)?
            .filter(|goal| goal.status == GoalStatus::Active)
            .map(|goal| {
                GoalProjection::from_active(&goal).map_err(|_| {
                    LedgerError::Invariant("active goal state is unsafe to project".into())
                })
            })
            .transpose()
    }
}

fn goal_id_for(conversation_id: &AgentChatConversationId, request_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"gent-goal-v2\0");
    digest.update(conversation_id.0.as_bytes());
    digest.update([0]);
    digest.update(request_id.as_bytes());
    format!("goal-{}", &hex::encode(digest.finalize())[..32])
}

fn contention() -> RuntimeError {
    RuntimeError::Ledger(LedgerError::Invariant(
        "goal changed concurrently too often".into(),
    ))
}

#[cfg(test)]
#[path = "goal_projection_tests.rs"]
mod tests;
