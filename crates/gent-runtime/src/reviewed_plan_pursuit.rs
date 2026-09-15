use gent_ports::{AgentChatPromptLedger, LedgerError, ReviewedPlanLedger};
use gent_types::{
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatRequestId, HostEpoch,
    MAX_PLAN_CONTENT_BYTES, PlanArtifact, PlanImplementation, PlanRevision, PlanStatus, PlanTurn,
    ReceiptId, ReviewedPlanId, bounded_text,
};
use sha2::{Digest, Sha256};

use super::{ReviewedPlanAuthority, ReviewedPlanService};
use crate::{GoalContinuationAdmission, GoalContinuationWake, RuntimeError};

impl<L: ReviewedPlanLedger + AgentChatPromptLedger> ReviewedPlanService<L> {
    /// # Errors
    /// Returns an error only when durable ingress is closed; other failures are reported.
    pub fn pursue(
        &self,
        host_epoch: HostEpoch,
        admission: &mut dyn GoalContinuationAdmission,
    ) -> Result<Vec<String>, RuntimeError> {
        if self.authority != ReviewedPlanAuthority::Approved {
            return Ok(Vec::new());
        }
        let mut failures = Vec::new();
        for turn in self.ledger.plan_turns_awaiting_review()? {
            if let Err(error) = self.ingest_turn(&turn) {
                failures.push(error.to_string());
            }
        }
        for implementation in self.ledger.implementations_awaiting_prompt()? {
            match self.save_implementation_prompt(&implementation, host_epoch) {
                Ok(wake) => {
                    if let Err(reason) = admission.admit(&wake) {
                        failures.push(reason);
                    }
                }
                Err(RuntimeError::Ledger(error @ LedgerError::IngressClosed { .. })) => {
                    return Err(error.into());
                }
                Err(error) => failures.push(error.to_string()),
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

    fn save_implementation_prompt(
        &self,
        implementation: &PlanImplementation,
        host_epoch: HostEpoch,
    ) -> Result<GoalContinuationWake, RuntimeError> {
        let request_id = digest_id("plan-implementation", &[&implementation.idempotency_key]);
        let saved = self.ledger.save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(request_id.clone()),
            receipt_id: ReceiptId(request_id),
            host_epoch,
            conversation_id: implementation.plan.conversation_id.clone(),
            disposition: AgentChatPromptDisposition::Send,
            text: implementation_prompt(&implementation.plan.content),
            attachment_ids: Vec::new(),
            tool_source_ids: Vec::new(),
        })?;
        if saved.run_id != implementation.implementation_run_id {
            return Err(RuntimeError::Ledger(LedgerError::Invariant(
                "reviewed plan implementation run is no longer current".into(),
            )));
        }
        Ok(GoalContinuationWake {
            conversation_id: gent_types::AgentChatConversationId(
                saved.message.conversation_id.clone(),
            ),
            run_id: saved.run_id,
            receipt_id: saved.receipt.receipt_id,
            message_id: saved.message.message_id,
        })
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

fn implementation_prompt(plan: &str) -> String {
    format!(
        "Implement the plan the user approved below. Work through it step by step and summarize what changed when you are done.\n\n<approved-plan>\n{plan}\n</approved-plan>"
    )
}
