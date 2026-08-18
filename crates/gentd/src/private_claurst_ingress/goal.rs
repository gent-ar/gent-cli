//! Gent-owned active-goal projection for the private Claurst bridge.

use gent_ports::{ActiveGoalResolver, ClaurstGoalProjection, ClaurstSourceId, GoalLedger};
use gent_runtime::{GoalAuthority, GoalService, RuntimeError};
use gent_types::AgentChatConversationId;

pub(super) fn resolve<L>(
    ledger: &L,
    conversation_id: &AgentChatConversationId,
    run_id: &str,
    source_id: &ClaurstSourceId,
) -> Result<Option<ClaurstGoalProjection>, RuntimeError>
where
    L: Clone + std::fmt::Debug + GoalLedger,
{
    let goal = GoalService::new(ledger.clone(), GoalAuthority::Approved)
        .resolve_active_goal(&conversation_id.0, run_id)?;
    Ok(goal.map(|goal| ClaurstGoalProjection {
        run_id: run_id.into(),
        source_id: source_id.clone(),
        goal,
    }))
}
