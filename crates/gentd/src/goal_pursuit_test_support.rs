use gent_ports::{AgentChatPromptDispatchLedger, GoalLedger};
use gent_runtime::{
    GoalAuthority, GoalContinuationAdmission, GoalContinuationWake, GoalPursuitService,
    GoalPursuitTick, GoalResult, GoalService,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationId, AgentChatPromptDisposition, GoalRecord, GoalTurnObservation, HostEpoch,
};

use crate::agent_chat_api::{PromptCommitWake, PromptWake};
use crate::ordinary_lifecycle_router::OrdinaryPublicLifecycleRouter;

pub(crate) struct RouterAdmission<'a> {
    pub(crate) ledger: &'a SqliteLedger,
    pub(crate) router: &'a mut OrdinaryPublicLifecycleRouter<SqliteLedger>,
    pub(crate) epoch: HostEpoch,
}

impl GoalContinuationAdmission for RouterAdmission<'_> {
    fn admit(&mut self, wake: &GoalContinuationWake) -> Result<(), String> {
        self.ledger
            .release_agent_chat_prompt_after_readiness(&wake.message_id, &wake.run_id, self.epoch)
            .map_err(|error| error.to_string())?;
        self.router
            .wake_after_prompt_commit(PromptWake {
                conversation_id: wake.conversation_id.clone(),
                run_id: wake.run_id.clone(),
                receipt_id: wake.receipt_id.clone(),
                disposition: AgentChatPromptDisposition::Send,
            })
            .map_err(|error| format!("{error:?}"))
    }
}

pub(crate) struct GoalDriver {
    pub(crate) goals: GoalService<SqliteLedger>,
    pub(crate) pursuit: GoalPursuitService<SqliteLedger>,
    ledger: SqliteLedger,
    conversation: AgentChatConversationId,
}

impl GoalDriver {
    pub(crate) fn new(ledger: &SqliteLedger, conversation: &AgentChatConversationId) -> Self {
        Self {
            goals: GoalService::new(ledger.clone(), GoalAuthority::Approved),
            pursuit: GoalPursuitService::new(ledger.clone()),
            ledger: ledger.clone(),
            conversation: conversation.clone(),
        }
    }

    pub(crate) fn set(&self, objective: &str, epoch: HostEpoch) -> GoalRecord {
        self.set_with_budget(objective, None, epoch)
    }

    pub(crate) fn set_with_budget(
        &self,
        objective: &str,
        token_budget: Option<u64>,
        epoch: HostEpoch,
    ) -> GoalRecord {
        let GoalResult::Goal(Some(goal)) = self
            .goals
            .set(
                "goal-request",
                &self.conversation,
                objective.into(),
                token_budget,
                epoch,
                1_000,
            )
            .unwrap()
        else {
            panic!("goal was not set");
        };
        goal
    }

    pub(crate) fn current(&self) -> GoalRecord {
        self.ledger
            .current_goal(&self.conversation.0)
            .unwrap()
            .unwrap()
    }

    pub(crate) fn turns(&self) -> Vec<GoalTurnObservation> {
        self.ledger.goal_turns(&self.conversation.0, 0).unwrap()
    }

    pub(crate) fn idle(&self) -> bool {
        self.turns().iter().all(|turn| turn.phase.is_terminal())
    }

    pub(crate) fn tick(
        &self,
        router: &mut OrdinaryPublicLifecycleRouter<SqliteLedger>,
        epoch: HostEpoch,
        now: u64,
    ) -> GoalPursuitTick {
        self.pursuit
            .tick(
                epoch,
                now,
                &mut RouterAdmission {
                    ledger: &self.ledger,
                    router,
                    epoch,
                },
            )
            .unwrap()
    }
}
