//! Daemon mapping for provider-neutral, durable `/goal` frames.

use gent_core::GoalRejection;
use gent_protocol::{GoalFrame, GoalRejectionCode};
use gent_runtime::{GoalControl, GoalResult, GoalService};
use gent_store::SqliteLedger;
use gent_types::{AgentChatConversationId, HostEpoch};

/// Handles one finite goal frame without involving provider lifecycle code.
pub(crate) fn exchange(
    service: &GoalService<SqliteLedger>,
    host_epoch: HostEpoch,
    frame: GoalFrame,
) -> Result<GoalFrame, String> {
    let now = crate::startup::unix_seconds();
    match frame {
        GoalFrame::Set {
            request_id,
            conversation_id,
            objective,
            token_budget,
        } => {
            let result = service
                .set(
                    &request_id,
                    &conversation_id,
                    objective,
                    token_budget,
                    host_epoch,
                    now,
                )
                .map_err(|error| error.to_string())?;
            reply(request_id, conversation_id, result)
        }
        GoalFrame::Pause {
            request_id,
            conversation_id,
            goal_id,
            expected_revision,
        } => control(
            service,
            (request_id, conversation_id, goal_id, expected_revision),
            GoalControl::Pause,
            host_epoch,
            now,
        ),
        GoalFrame::Resume {
            request_id,
            conversation_id,
            goal_id,
            expected_revision,
        } => control(
            service,
            (request_id, conversation_id, goal_id, expected_revision),
            GoalControl::Resume,
            host_epoch,
            now,
        ),
        GoalFrame::Clear {
            request_id,
            conversation_id,
            goal_id,
            expected_revision,
        } => control(
            service,
            (request_id, conversation_id, goal_id, expected_revision),
            GoalControl::Clear,
            host_epoch,
            now,
        ),
        GoalFrame::Read {
            request_id,
            conversation_id,
        } => {
            let result = service
                .current(&conversation_id)
                .map_err(|error| error.to_string())?;
            reply(request_id, conversation_id, result)
        }
        GoalFrame::Report {
            request_id,
            goal_id,
            outcome,
            note,
        } => {
            let result = service
                .report(&goal_id, outcome, note, host_epoch, now)
                .map_err(|error| error.to_string())?;
            let conversation_id = match &result {
                GoalResult::Goal(Some(goal)) => goal.binding.conversation_id.clone(),
                _ => AgentChatConversationId(String::new()),
            };
            reply(request_id, conversation_id, result)
        }
        GoalFrame::Goal { .. } | GoalFrame::Rejected { .. } => {
            Err("goal response frames are server-only".into())
        }
    }
}

fn control(
    service: &GoalService<SqliteLedger>,
    (request_id, conversation_id, goal_id, expected_revision): (
        String,
        AgentChatConversationId,
        String,
        u64,
    ),
    control: GoalControl,
    host_epoch: HostEpoch,
    now: u64,
) -> Result<GoalFrame, String> {
    let result = service
        .control(
            &conversation_id,
            &goal_id,
            expected_revision,
            control,
            host_epoch,
            now,
        )
        .map_err(|error| error.to_string())?;
    reply(request_id, conversation_id, result)
}

fn reply(
    request_id: String,
    conversation_id: AgentChatConversationId,
    result: GoalResult,
) -> Result<GoalFrame, String> {
    match result {
        GoalResult::Goal(goal) => Ok(GoalFrame::Goal {
            request_id,
            conversation_id,
            goal,
        }),
        GoalResult::Rejected(rejection) => Ok(GoalFrame::Rejected {
            request_id,
            code: code(rejection),
        }),
        GoalResult::DeniedObserver => {
            Err("goals are unavailable while gentd is observer-disabled".into())
        }
    }
}

const fn code(rejection: GoalRejection) -> GoalRejectionCode {
    match rejection {
        GoalRejection::InvalidValue => GoalRejectionCode::GoalInvalid,
        GoalRejection::Missing => GoalRejectionCode::GoalMissing,
        GoalRejection::RevisionMismatch => GoalRejectionCode::GoalRevisionMismatch,
        GoalRejection::NotActive => GoalRejectionCode::GoalNotActive,
        GoalRejection::NotResumable => GoalRejectionCode::GoalNotResumable,
        GoalRejection::BudgetExhausted => GoalRejectionCode::GoalBudgetExhausted,
        GoalRejection::NoActiveTurn => GoalRejectionCode::GoalNoActiveTurn,
    }
}
