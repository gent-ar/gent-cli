use gent_ports::{LedgerError, ReviewedPlanLedger};
use gent_types::{
    MAX_PLAN_CONTENT_BYTES, PlanArtifact, PlanRevision, PlanStatus, PlanTurn, ReviewedPlanId,
    bounded_text,
};
use sha2::{Digest, Sha256};

use super::{ReviewedPlanAuthority, ReviewedPlanService};
use crate::RuntimeError;

impl<L: ReviewedPlanLedger> ReviewedPlanService<L> {
    /// # Errors
    /// Returns an error only when durable ingress is closed; other failures are reported.
    pub fn pursue(&self) -> Result<Vec<String>, RuntimeError> {
        if self.authority != ReviewedPlanAuthority::Approved {
            return Ok(Vec::new());
        }
        let mut failures = Vec::new();
        for turn in self.ledger.plan_turns_awaiting_review()? {
            if let Err(error) = self.ingest_turn(&turn) {
                failures.push(error.to_string());
            }
        }
        Ok(failures)
    }

    fn ingest_turn(&self, turn: &PlanTurn) -> Result<(), RuntimeError> {
        let plan_id = ReviewedPlanId(digest_id(
            "plan",
            &[&turn.conversation_id.0, &turn.run_id.0],
        ));
        let revision = self
            .ledger
            .reviewed_plan(&turn.conversation_id.0, &plan_id)?
            .map_or(1, |current| current.revision.0.saturating_add(1));
        let content = bounded_text(&turn.content, MAX_PLAN_CONTENT_BYTES).into_owned();
        let plan = PlanArtifact {
            plan_id,
            conversation_id: turn.conversation_id.clone(),
            source_run_id: turn.run_id.clone(),
            source_turn_id: turn.turn_id.clone(),
            revision: PlanRevision(revision),
            content_digest_sha256: PlanArtifact::content_digest(&content),
            status: PlanStatus::ReadyForReview,
            content,
        };
        plan.validate()
            .map_err(|error| RuntimeError::Ledger(LedgerError::Invariant(error.to_string())))?;
        Ok(self.ledger.save_trusted_plan(&plan)?)
    }
}

fn digest_id(prefix: &str, parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    format!("{prefix}-{:x}", hash.finalize())
}
