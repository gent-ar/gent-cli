//! Daemon mapping for user-owned decisions over daemon-trusted plan artifacts.

use gent_ports::ReviewedPlanLedger;
use gent_protocol::ReviewedPlanFrame;
use gent_runtime::{ReviewedPlanResult, ReviewedPlanService};

pub(crate) fn exchange<L: ReviewedPlanLedger>(
    service: &ReviewedPlanService<L>,
    frame: ReviewedPlanFrame,
) -> Result<ReviewedPlanFrame, String> {
    match frame {
        ReviewedPlanFrame::ReviewRead {
            request_id,
            conversation_id,
            plan_id,
        } => match service
            .review(&conversation_id.0, &plan_id)
            .map_err(|error| error.to_string())?
        {
            ReviewedPlanResult::Plan(plan) => Ok(ReviewedPlanFrame::Review { request_id, plan }),
            result => rejected(&result),
        },
        ReviewedPlanFrame::StartImplementation { request } => {
            let request_id = request.request_id.0.clone();
            match service.start(&request).map_err(|error| error.to_string())? {
                ReviewedPlanResult::Started(result) => {
                    Ok(ReviewedPlanFrame::StartedImplementation { request_id, result })
                }
                result => rejected(&result),
            }
        }
        ReviewedPlanFrame::Reject {
            request_id,
            plan_id,
            plan_revision,
            plan_content_digest_sha256,
        } => match service
            .reject(&plan_id, plan_revision, &plan_content_digest_sha256)
            .map_err(|error| error.to_string())?
        {
            ReviewedPlanResult::Rejected => Ok(ReviewedPlanFrame::Rejected {
                request_id,
                plan_id,
                plan_revision,
            }),
            result => rejected(&result),
        },
        ReviewedPlanFrame::Review { .. }
        | ReviewedPlanFrame::StartedImplementation { .. }
        | ReviewedPlanFrame::Rejected { .. } => {
            Err("reviewed-plan response frames are server-only".into())
        }
    }
}

fn rejected(result: &ReviewedPlanResult) -> Result<ReviewedPlanFrame, String> {
    let message = match result {
        ReviewedPlanResult::DeniedObserver => {
            "reviewed plans are unavailable while gentd is observer-disabled"
        }
        ReviewedPlanResult::Plan(_)
        | ReviewedPlanResult::Started(_)
        | ReviewedPlanResult::Rejected => "reviewed-plan request had an invalid result",
    };
    Err(message.into())
}
