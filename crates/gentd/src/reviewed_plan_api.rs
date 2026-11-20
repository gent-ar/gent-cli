//! Daemon mapping for user-owned decisions over daemon-trusted plan artifacts.

use gent_ports::{LedgerError, ReviewedPlanLedger};
use gent_protocol::ReviewedPlanFrame;
use gent_runtime::{ReviewedPlanResult, ReviewedPlanService, RuntimeError};
use gent_store::SqliteLedger;

use crate::agent_chat_intent_error::AgentChatIntentError;
use crate::goal_pursuit_host::IngressAdmission;
use crate::ordinary_lifecycle_cadence::OrdinaryPromptIngress;

pub(crate) fn rejected(error: impl std::fmt::Display) -> AgentChatIntentError {
    AgentChatIntentError::coded("reviewedPlanRejected", &error.to_string())
}

fn failed(error: RuntimeError) -> AgentChatIntentError {
    match error {
        RuntimeError::Ledger(LedgerError::Rejected(rejection)) => rejection.into(),
        error => rejected(error),
    }
}

pub(crate) fn exchange<L: ReviewedPlanLedger>(
    service: &ReviewedPlanService<L>,
    frame: ReviewedPlanFrame,
    ingress: Option<OrdinaryPromptIngress<SqliteLedger>>,
) -> Result<ReviewedPlanFrame, AgentChatIntentError> {
    match frame {
        ReviewedPlanFrame::ReviewRead {
            request_id,
            conversation_id,
            plan_id,
        } => match service
            .review(&conversation_id.0, plan_id.as_ref())
            .map_err(failed)?
        {
            ReviewedPlanResult::Plan(plan) => Ok(ReviewedPlanFrame::Review { request_id, plan }),
            result => Err(unexpected(&result)),
        },
        ReviewedPlanFrame::StartImplementation { request } => {
            let request_id = request.request_id.0.clone();
            let mut admission = ingress
                .map(IngressAdmission)
                .ok_or_else(|| rejected("agent-chat provider lifecycle is not configured"))?;
            match service.start(&request, &mut admission).map_err(failed)? {
                ReviewedPlanResult::Started(result) => {
                    Ok(ReviewedPlanFrame::StartedImplementation { request_id, result })
                }
                result => Err(unexpected(&result)),
            }
        }
        ReviewedPlanFrame::Reject {
            request_id,
            plan_id,
            plan_revision,
            plan_content_digest_sha256,
        } => match service
            .reject(&plan_id, plan_revision, &plan_content_digest_sha256)
            .map_err(failed)?
        {
            ReviewedPlanResult::Rejected => Ok(ReviewedPlanFrame::Rejected {
                request_id,
                plan_id,
                plan_revision,
            }),
            result => Err(unexpected(&result)),
        },
        ReviewedPlanFrame::Review { .. }
        | ReviewedPlanFrame::StartedImplementation { .. }
        | ReviewedPlanFrame::Rejected { .. } => {
            Err(rejected("reviewed-plan response frames are server-only"))
        }
    }
}

fn unexpected(result: &ReviewedPlanResult) -> AgentChatIntentError {
    rejected(match result {
        ReviewedPlanResult::DeniedObserver => {
            "reviewed plans are unavailable while gentd is observer-disabled"
        }
        ReviewedPlanResult::Plan(_)
        | ReviewedPlanResult::Started(_)
        | ReviewedPlanResult::Rejected => "reviewed-plan request had an invalid result",
    })
}

#[cfg(test)]
mod tests {
    use gent_ports::LedgerError;
    use gent_runtime::RuntimeError;
    use gent_types::AgentChatRejection;

    #[test]
    fn a_typed_plan_rejection_keeps_its_code_and_other_failures_stay_reviewed_plan_rejected() {
        let typed = super::failed(RuntimeError::Ledger(LedgerError::Rejected(
            AgentChatRejection::PlanNotReviewable,
        )));
        assert_eq!(
            (typed.code, typed.message.as_str()),
            (
                "planNotReviewable",
                "this plan revision is no longer awaiting review"
            )
        );
        let untyped = super::failed(RuntimeError::Ledger(LedgerError::Storage("disk".into())));
        assert_eq!(untyped.code, "reviewedPlanRejected");
    }
}
