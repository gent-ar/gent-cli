use std::sync::{Arc, Mutex};

use gent_ports::{ActiveGoalResolver, GoalLedger, GoalWrite, LedgerError};
use gent_types::{
    AgentChatConversationId, AgentChatRunId, GOAL_SCHEMA_VERSION, GoalBinding, GoalRecord,
    GoalStatus,
};

use crate::{GoalAuthority, GoalService};

#[derive(Debug)]
struct SnapshotLedger(Arc<Mutex<Vec<GoalRecord>>>);

impl GoalLedger for SnapshotLedger {
    fn find_goal(&self, _: &GoalBinding) -> Result<Option<GoalRecord>, LedgerError> {
        unreachable!("projection only reads the conversation snapshot")
    }
    fn create_goal(&self, _: &GoalRecord) -> Result<GoalWrite, LedgerError> {
        unreachable!("projection does not write")
    }
    fn replace_goal(&self, _: &GoalRecord, _: &GoalRecord) -> Result<GoalWrite, LedgerError> {
        unreachable!("projection does not write")
    }
    fn conversation_goals(&self, _: &str) -> Result<Vec<GoalRecord>, LedgerError> {
        Ok(self.0.lock().unwrap().clone())
    }
}

fn goal() -> GoalRecord {
    GoalRecord {
        schema_version: GOAL_SCHEMA_VERSION,
        binding: GoalBinding {
            goal_id: "goal-1".into(),
            conversation_id: AgentChatConversationId("conversation-1".into()),
            run_id: AgentChatRunId("run-1".into()),
        },
        revision: 1,
        status: GoalStatus::Active,
        summary: "Finish the terminal workflow".into(),
    }
}

#[test]
fn approved_projection_reloads_each_follow_up_and_omits_stale_or_completed_goals() {
    let goals = Arc::new(Mutex::new(vec![goal()]));
    let service = GoalService::new(SnapshotLedger(Arc::clone(&goals)), GoalAuthority::Approved);
    assert_eq!(
        service
            .resolve_active_goal("conversation-1", "run-1")
            .unwrap()
            .unwrap()
            .revision(),
        1
    );
    goals.lock().unwrap()[0].revision = 4;
    assert_eq!(
        service
            .resolve_active_goal("conversation-1", "run-1")
            .unwrap()
            .unwrap()
            .revision(),
        4
    );
    goals.lock().unwrap()[0].status = GoalStatus::Completed;
    assert_eq!(
        service
            .resolve_active_goal("conversation-1", "run-1")
            .unwrap(),
        None
    );
    goals.lock().unwrap()[0] = GoalRecord {
        binding: GoalBinding {
            run_id: AgentChatRunId("other-run".into()),
            ..goal().binding
        },
        ..goal()
    };
    assert_eq!(
        service
            .resolve_active_goal("conversation-1", "run-1")
            .unwrap(),
        None
    );
}
