//! Durable ownership boundary for trusted reviewed-plan artifacts and approvals.

use gent_types::{
    PlanArtifact, PlanRevision, ReviewedPlanId, StartImplementationRequest,
    StartImplementationResult,
};

use crate::LedgerError;

/// Persists daemon-normalized plans and atomically reserves reviewed implementation children.
///
/// No public client receives the artifact-ingress operation: only daemon-owned normalized
/// provider ingress may call it after its own authority checks.
pub trait ReviewedPlanLedger: Send + Sync {
    /// Stores one trusted immutable plan revision, superseding an earlier revision if present.
    ///
    /// # Errors
    /// Returns an error when its source boundary is not durable or the revision conflicts.
    fn save_trusted_plan(&self, plan: &PlanArtifact) -> Result<(), LedgerError>;

    /// Reads the current reviewed artifact for one conversation and plan identity.
    ///
    /// # Errors
    /// Returns an error when the durable projection cannot be read.
    fn reviewed_plan(
        &self,
        conversation_id: &str,
        plan_id: &ReviewedPlanId,
    ) -> Result<Option<PlanArtifact>, LedgerError>;

    /// Rechecks all approval fences and atomically reserves an immutable implementation child.
    ///
    /// # Errors
    /// Returns an error when any exact revision, policy, epoch, receipt, or parent fence fails.
    fn approve_reviewed_plan(
        &self,
        request: &StartImplementationRequest,
    ) -> Result<StartImplementationResult, LedgerError>;

    /// Atomically rejects exactly one still-reviewable plan revision.
    ///
    /// # Errors
    /// Returns an error when the revision/digest does not identify a reviewable trusted plan.
    fn reject_reviewed_plan(
        &self,
        plan_id: &ReviewedPlanId,
        revision: PlanRevision,
        content_digest_sha256: &str,
    ) -> Result<(), LedgerError>;
}
