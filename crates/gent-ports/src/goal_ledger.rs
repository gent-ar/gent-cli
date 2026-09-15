//! Durable boundary for provider-neutral user-authored conversation goals.

use gent_types::{GoalRecord, GoalTurnObservation, HostEpoch};

use crate::LedgerError;

pub const MAX_GOAL_TURN_OBSERVATIONS: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GoalWrite {
    Updated(GoalRecord),
    Current(Option<GoalRecord>),
}

pub trait GoalLedger: Send + Sync {
    fn current_goal(&self, conversation_id: &str) -> Result<Option<GoalRecord>, LedgerError>;

    fn find_goal(&self, goal_id: &str) -> Result<Option<GoalRecord>, LedgerError>;

    fn active_goals(&self) -> Result<Vec<GoalRecord>, LedgerError>;

    fn create_goal(
        &self,
        expected_current: Option<&GoalRecord>,
        replaced: Option<&GoalRecord>,
        goal: &GoalRecord,
        host_epoch: HostEpoch,
    ) -> Result<GoalWrite, LedgerError>;

    fn replace_goal(
        &self,
        expected: &GoalRecord,
        next: &GoalRecord,
        host_epoch: HostEpoch,
    ) -> Result<GoalWrite, LedgerError>;

    fn goal_turns(
        &self,
        conversation_id: &str,
        after_ordinal: u64,
    ) -> Result<Vec<GoalTurnObservation>, LedgerError>;

    fn latest_turn_ordinal(&self, conversation_id: &str) -> Result<u64, LedgerError>;
}
