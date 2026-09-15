use std::{sync::Arc, time::Duration};

use gent_runtime::{
    GoalContinuationAdmission, GoalContinuationWake, GoalPursuitService, GoalPursuitTick,
    ReviewedPlanAuthority, ReviewedPlanService,
};
use gent_store::SqliteLedger;
use gent_types::{AgentChatPromptDisposition, HostEpoch};

use crate::agent_chat_api::{PromptCommitWake, PromptWake};
use crate::ordinary_lifecycle_cadence::OrdinaryPromptIngress;
use crate::ordinary_lifecycle_control::OrdinaryLifecycleControl;

pub(crate) const GOAL_PURSUIT_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Debug)]
pub(crate) struct GoalPursuitHost {
    service: Arc<GoalPursuitService<SqliteLedger>>,
    plans: Arc<ReviewedPlanService<SqliteLedger>>,
    ingress: OrdinaryPromptIngress<SqliteLedger>,
    host_epoch: HostEpoch,
}

struct IngressAdmission(OrdinaryPromptIngress<SqliteLedger>);

impl GoalContinuationAdmission for IngressAdmission {
    fn admit(&mut self, wake: &GoalContinuationWake) -> Result<(), String> {
        self.0.wake_after_prompt_commit(PromptWake {
            conversation_id: wake.conversation_id.clone(),
            run_id: wake.run_id.clone(),
            receipt_id: wake.receipt_id.clone(),
            disposition: AgentChatPromptDisposition::Send,
        })
    }
}

impl GoalPursuitHost {
    pub(crate) fn new(
        ledger: SqliteLedger,
        ingress: OrdinaryPromptIngress<SqliteLedger>,
        host_epoch: HostEpoch,
    ) -> Self {
        Self {
            plans: Arc::new(ReviewedPlanService::new(
                ledger.clone(),
                ReviewedPlanAuthority::Approved,
            )),
            service: Arc::new(GoalPursuitService::new(ledger)),
            ingress,
            host_epoch,
        }
    }

    pub(crate) async fn run(self, control: OrdinaryLifecycleControl) {
        if control.wait_until_ready().await.is_err() {
            return;
        }
        loop {
            tokio::select! {
                () = control.shutdown_requested() => return,
                () = tokio::time::sleep(GOAL_PURSUIT_INTERVAL) => {}
            }
            let host = self.clone();
            match tokio::task::spawn_blocking(move || host.tick()).await {
                Ok(Ok(tick)) => {
                    for failure in tick.failures {
                        eprintln!("goal pursuit could not continue a goal: {failure}");
                    }
                }
                Ok(Err(error)) => eprintln!("goal pursuit tick failed: {error}"),
                Err(_) => eprintln!("goal pursuit tick task failed"),
            }
        }
    }

    pub(crate) fn tick(&self) -> Result<GoalPursuitTick, String> {
        let Ok(_permit) = self.ingress.acquire_prompt() else {
            return Ok(GoalPursuitTick::default());
        };
        let mut admission = IngressAdmission(self.ingress.clone());
        let mut tick = self
            .service
            .tick(
                self.host_epoch,
                crate::startup::unix_seconds(),
                &mut admission,
            )
            .map_err(|error| error.to_string())?;
        tick.failures.extend(
            self.plans
                .pursue(self.host_epoch, &mut admission)
                .map_err(|error| error.to_string())?,
        );
        Ok(tick)
    }
}

#[cfg(all(test, unix))]
#[path = "goal_pursuit_test_support.rs"]
pub(crate) mod test_support;
