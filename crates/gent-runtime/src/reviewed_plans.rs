//! Authority-gated reviewed-plan coordination over the durable ledger port.

use gent_ports::ReviewedPlanLedger;
use gent_types::{
    PlanArtifact, PlanRevision, ReviewedPlanId, StartImplementationRequest,
    StartImplementationResult,
};

use crate::{GoalContinuationAdmission, GoalContinuationWake, RuntimeError};

/// Explicit composition authority for trusted-plan ingress and review decisions.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ReviewedPlanAuthority {
    /// The default daemon cannot receive plans or settle review choices.
    #[default]
    Observer,
    /// Reserved for a future evidence-approved daemon composition.
    Approved,
}

/// The observable outcome of one reviewed-plan operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReviewedPlanResult {
    DeniedObserver,
    Plan(Option<PlanArtifact>),
    Started(StartImplementationResult),
    Rejected,
}

/// Coordinates trusted-plan storage through one typed port without provider access.
#[derive(Clone, Debug)]
pub struct ReviewedPlanService<L> {
    ledger: L,
    authority: ReviewedPlanAuthority,
}

impl<L> ReviewedPlanService<L> {
    /// Creates the inert observer service unless a future composition explicitly approves it.
    #[must_use]
    pub fn new(ledger: L, authority: ReviewedPlanAuthority) -> Self {
        Self { ledger, authority }
    }
}

impl<L: ReviewedPlanLedger> ReviewedPlanService<L> {
    /// Saves only daemon-normalized provider plan artifacts; no IPC client may call this method.
    ///
    /// # Errors
    /// Returns an error only after explicitly approved composition reaches durable storage.
    pub fn ingest_trusted(&self, plan: &PlanArtifact) -> Result<ReviewedPlanResult, RuntimeError> {
        if self.authority != ReviewedPlanAuthority::Approved {
            return Ok(ReviewedPlanResult::DeniedObserver);
        }
        self.ledger.save_trusted_plan(plan)?;
        Ok(ReviewedPlanResult::Plan(Some(plan.clone())))
    }

    /// Reads a trusted artifact for an IPC review request.
    ///
    /// # Errors
    /// Returns an error only after explicitly approved composition reaches durable storage.
    pub fn review(
        &self,
        conversation_id: &str,
        plan_id: Option<&ReviewedPlanId>,
    ) -> Result<ReviewedPlanResult, RuntimeError> {
        if self.authority != ReviewedPlanAuthority::Approved {
            return Ok(ReviewedPlanResult::DeniedObserver);
        }
        Ok(ReviewedPlanResult::Plan(match plan_id {
            Some(plan_id) => self.ledger.reviewed_plan(conversation_id, plan_id)?,
            None => self.ledger.current_conversation_plan(conversation_id)?,
        }))
    }

    /// Atomically rechecks and reserves the implementation child through the ledger.
    ///
    /// # Errors
    /// Returns an error only after explicitly approved composition reaches durable storage.
    pub fn start(
        &self,
        request: &StartImplementationRequest,
        admission: &mut dyn GoalContinuationAdmission,
    ) -> Result<ReviewedPlanResult, RuntimeError> {
        if self.authority != ReviewedPlanAuthority::Approved {
            return Ok(ReviewedPlanResult::DeniedObserver);
        }
        let started = self.ledger.approve_reviewed_plan(request)?;
        admission
            .admit(&GoalContinuationWake {
                conversation_id: request.conversation_id.clone(),
                run_id: started.prompt.run_id,
                receipt_id: started.prompt.receipt.receipt_id,
                message_id: started.prompt.message.message_id,
            })
            .map_err(|reason| RuntimeError::Port(gent_ports::PortError::Unavailable(reason)))?;
        Ok(ReviewedPlanResult::Started(started.result))
    }

    /// Rejects the exact trusted plan revision without deleting durable history.
    ///
    /// # Errors
    /// Returns an error only after explicitly approved composition reaches durable storage.
    pub fn reject(
        &self,
        plan_id: &ReviewedPlanId,
        revision: PlanRevision,
        digest: &str,
    ) -> Result<ReviewedPlanResult, RuntimeError> {
        if self.authority != ReviewedPlanAuthority::Approved {
            return Ok(ReviewedPlanResult::DeniedObserver);
        }
        self.ledger
            .reject_reviewed_plan(plan_id, revision, digest)?;
        Ok(ReviewedPlanResult::Rejected)
    }
}

#[path = "reviewed_plan_pursuit.rs"]
mod pursuit;

#[cfg(test)]
mod tests {
    use super::{ReviewedPlanAuthority, ReviewedPlanResult, ReviewedPlanService};

    #[test]
    fn observer_has_no_reviewed_plan_storage_or_approval_path() {
        let service = ReviewedPlanService::new((), ReviewedPlanAuthority::Observer);
        assert!(matches!(service.authority, ReviewedPlanAuthority::Observer));
        let _ = ReviewedPlanResult::DeniedObserver;
    }
}
