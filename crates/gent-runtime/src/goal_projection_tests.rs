use gent_ports::{ActiveGoalResolver, GoalLedger, GoalWrite, LedgerError};
use gent_types::{
    AgentChatConversationId, GoalRecord, GoalReportOutcome, GoalTurnObservation, HostEpoch,
};

use super::{GoalAuthority, GoalControl, GoalResult, GoalService};

#[derive(Debug)]
struct PanicLedger;

impl GoalLedger for PanicLedger {
    fn current_goal(&self, _: &str) -> Result<Option<GoalRecord>, LedgerError> {
        panic!("observer goal read reached the ledger")
    }
    fn find_goal(&self, _: &str) -> Result<Option<GoalRecord>, LedgerError> {
        panic!("observer goal read reached the ledger")
    }
    fn active_goals(&self) -> Result<Vec<GoalRecord>, LedgerError> {
        panic!("observer goal read reached the ledger")
    }
    fn create_goal(
        &self,
        _: Option<&GoalRecord>,
        _: Option<&GoalRecord>,
        _: &GoalRecord,
        _: HostEpoch,
    ) -> Result<GoalWrite, LedgerError> {
        panic!("observer goal write reached the ledger")
    }
    fn replace_goal(
        &self,
        _: &GoalRecord,
        _: &GoalRecord,
        _: HostEpoch,
    ) -> Result<GoalWrite, LedgerError> {
        panic!("observer goal write reached the ledger")
    }
    fn goal_turns(&self, _: &str, _: u64) -> Result<Vec<GoalTurnObservation>, LedgerError> {
        panic!("observer goal read reached the ledger")
    }
    fn latest_turn_ordinal(&self, _: &str) -> Result<u64, LedgerError> {
        panic!("observer goal read reached the ledger")
    }
}

#[test]
fn observer_has_no_goal_read_write_or_projection_path() {
    let service = GoalService::new(PanicLedger, GoalAuthority::Observer);
    let conversation = AgentChatConversationId("conversation-1".into());
    assert_eq!(
        service
            .set(
                "request-1",
                &conversation,
                "Ship".into(),
                None,
                HostEpoch(1),
                1
            )
            .unwrap(),
        GoalResult::DeniedObserver
    );
    assert_eq!(
        service
            .control(
                &conversation,
                "goal-1",
                1,
                GoalControl::Pause,
                HostEpoch(1),
                1
            )
            .unwrap(),
        GoalResult::DeniedObserver
    );
    assert_eq!(
        service
            .report("goal-1", GoalReportOutcome::Complete, None, HostEpoch(1), 1)
            .unwrap(),
        GoalResult::DeniedObserver
    );
    assert_eq!(service.stop(&conversation, HostEpoch(1), 1).unwrap(), None);
    assert_eq!(
        service.current(&conversation).unwrap(),
        GoalResult::DeniedObserver
    );
    assert_eq!(service.resolve_active_goal("conversation-1").unwrap(), None);
}
