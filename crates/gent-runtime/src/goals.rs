//! Observer-gated coordination of durable, provider-neutral user goals.

use gent_core::{
    ActiveGoalSelection, GoalControlContext, GoalControlEffect, GoalControlEvent,
    GoalControlRejection, GoalControlState, reduce_goal_control, select_active_goal,
};
use gent_ports::{ActiveGoalResolver, GoalLedger, GoalWrite, LedgerError};
use gent_types::{GoalBinding, GoalProjection, GoalRecord, GoalTransition};

use crate::RuntimeError;

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
    Missing,
    Goal(GoalRecord),
    Goals(Vec<GoalRecord>),
    Rejected(GoalControlRejection),
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
    /// Creates a user-authored goal only through approved daemon composition.
    ///
    /// # Errors
    /// Returns an error only after approved composition reaches durable storage.
    pub fn create(
        &self,
        context: &GoalControlContext,
        goal: GoalRecord,
    ) -> Result<GoalResult, RuntimeError> {
        if self.authority != GoalAuthority::Approved {
            return Ok(GoalResult::DeniedObserver);
        }
        let (_, effect) = reduce_goal_control(
            GoalControlState::default(),
            context,
            GoalControlEvent::Create(goal),
        );
        match effect {
            GoalControlEffect::Persist(candidate) => match self.ledger.create_goal(&candidate)? {
                GoalWrite::Created(goal) | GoalWrite::Updated(goal) | GoalWrite::Current(goal) => {
                    Ok(GoalResult::Goal(goal))
                }
            },
            GoalControlEffect::Unchanged(goal) => Ok(GoalResult::Goal(goal)),
            GoalControlEffect::Rejected(reason) => Ok(GoalResult::Rejected(reason)),
        }
    }

    /// Settles an active goal after reading and atomically rechecking its exact revision.
    ///
    /// # Errors
    /// Returns an error only after approved composition reaches durable storage.
    pub fn transition(
        &self,
        context: &GoalControlContext,
        request: GoalTransition,
    ) -> Result<GoalResult, RuntimeError> {
        if self.authority != GoalAuthority::Approved {
            return Ok(GoalResult::DeniedObserver);
        }
        let Some(current) = self.ledger.find_goal(&request.binding)? else {
            return Ok(GoalResult::Missing);
        };
        let (_, effect) = reduce_goal_control(
            GoalControlState::new(Some(current.clone())),
            context,
            GoalControlEvent::Transition(request),
        );
        match effect {
            GoalControlEffect::Persist(next) => match self.ledger.replace_goal(&current, &next)? {
                GoalWrite::Updated(goal) | GoalWrite::Created(goal) | GoalWrite::Current(goal) => {
                    Ok(GoalResult::Goal(goal))
                }
            },
            GoalControlEffect::Unchanged(goal) => Ok(GoalResult::Goal(goal)),
            GoalControlEffect::Rejected(reason) => Ok(GoalResult::Rejected(reason)),
        }
    }

    /// Reads one durable goal only through approved composition.
    ///
    /// # Errors
    /// Returns an error only after approved composition reaches durable storage.
    pub fn get(&self, binding: &GoalBinding) -> Result<GoalResult, RuntimeError> {
        if self.authority != GoalAuthority::Approved {
            return Ok(GoalResult::DeniedObserver);
        }
        Ok(self
            .ledger
            .find_goal(binding)?
            .map_or(GoalResult::Missing, GoalResult::Goal))
    }

    /// Lists current conversation goals only through approved composition.
    ///
    /// # Errors
    /// Returns an error only after approved composition reaches durable storage.
    pub fn list(&self, conversation_id: &str) -> Result<GoalResult, RuntimeError> {
        if self.authority != GoalAuthority::Approved {
            return Ok(GoalResult::DeniedObserver);
        }
        Ok(GoalResult::Goals(
            self.ledger.conversation_goals(conversation_id)?,
        ))
    }
}

impl<L: GoalLedger + std::fmt::Debug> ActiveGoalResolver for GoalService<L> {
    fn resolve_active_goal(
        &self,
        conversation_id: &str,
        run_id: &str,
    ) -> Result<Option<GoalProjection>, LedgerError> {
        if self.authority != GoalAuthority::Approved {
            return Ok(None);
        }
        match select_active_goal(
            &self.ledger.conversation_goals(conversation_id)?,
            conversation_id,
            run_id,
        ) {
            ActiveGoalSelection::None => Ok(None),
            ActiveGoalSelection::Goal(goal) => Ok(Some(goal)),
            ActiveGoalSelection::Rejected(_) => Err(LedgerError::Invariant(
                "active goal state is unsafe to project".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use gent_core::GoalControlContext;
    use gent_ports::{ActiveGoalResolver, GoalLedger, GoalWrite, LedgerError};
    use gent_types::{
        AgentChatConversationId, AgentChatRunId, GOAL_SCHEMA_VERSION, GoalBinding, GoalRecord,
        GoalStatus, GoalTransition, HostEpoch,
    };

    use super::{GoalAuthority, GoalResult, GoalService};

    #[derive(Debug)]
    struct PanicLedger;

    impl GoalLedger for PanicLedger {
        fn find_goal(&self, _: &GoalBinding) -> Result<Option<GoalRecord>, LedgerError> {
            panic!("observer goal read reached the ledger")
        }

        fn create_goal(&self, _: &GoalRecord) -> Result<GoalWrite, LedgerError> {
            panic!("observer goal create reached the ledger")
        }

        fn replace_goal(&self, _: &GoalRecord, _: &GoalRecord) -> Result<GoalWrite, LedgerError> {
            panic!("observer goal transition reached the ledger")
        }

        fn conversation_goals(&self, _: &str) -> Result<Vec<GoalRecord>, LedgerError> {
            panic!("observer goal list reached the ledger")
        }
    }

    fn binding() -> GoalBinding {
        GoalBinding {
            goal_id: "goal-1".into(),
            conversation_id: AgentChatConversationId("conversation-1".into()),
            run_id: AgentChatRunId("run-1".into()),
        }
    }

    fn context() -> GoalControlContext {
        GoalControlContext {
            conversation_id: "conversation-1".into(),
            run_id: "run-1".into(),
            host_epoch: HostEpoch(1),
        }
    }

    fn goal() -> GoalRecord {
        GoalRecord {
            schema_version: GOAL_SCHEMA_VERSION,
            binding: binding(),
            revision: 1,
            status: GoalStatus::Active,
            summary: "Finish the terminal workflow".into(),
        }
    }

    #[test]
    fn observer_has_no_goal_read_or_write_path() {
        let service = GoalService::new(PanicLedger, GoalAuthority::Observer);
        assert_eq!(
            service.create(&context(), goal()).unwrap(),
            GoalResult::DeniedObserver
        );
        assert_eq!(
            service
                .transition(
                    &context(),
                    GoalTransition {
                        binding: binding(),
                        expected_revision: 1,
                        host_epoch: HostEpoch(1),
                        next_status: GoalStatus::Completed,
                    },
                )
                .unwrap(),
            GoalResult::DeniedObserver
        );
        assert_eq!(service.get(&binding()).unwrap(), GoalResult::DeniedObserver);
        assert_eq!(
            service.list("conversation-1").unwrap(),
            GoalResult::DeniedObserver
        );
        assert_eq!(
            service
                .resolve_active_goal("conversation-1", "run-1")
                .unwrap(),
            None
        );
    }
}
