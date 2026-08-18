//! Daemon mapping for provider-neutral, durable `/goal` frames.

use gent_core::GoalControlContext;
use gent_protocol::GoalFrame;
use gent_runtime::{GoalResult, GoalService};
use gent_store::SqliteLedger;
use gent_types::HostEpoch;

/// Handles one finite goal frame without involving provider lifecycle code.
pub(crate) fn exchange(
    service: &GoalService<SqliteLedger>,
    host_epoch: HostEpoch,
    frame: GoalFrame,
) -> Result<GoalFrame, String> {
    match frame {
        GoalFrame::Create { request_id, goal } => {
            let context = context(&goal.binding, host_epoch);
            create_reply(
                request_id,
                service
                    .create(&context, goal)
                    .map_err(|error| error.to_string())?,
            )
        }
        GoalFrame::Transition {
            request_id,
            transition,
        } => {
            let context = context(&transition.binding, host_epoch);
            transition_reply(
                request_id,
                service
                    .transition(&context, transition)
                    .map_err(|error| error.to_string())?,
            )
        }
        GoalFrame::Read {
            request_id,
            binding,
        } => {
            let result = service.get(&binding).map_err(|error| error.to_string())?;
            match result {
                GoalResult::Goal(goal) => Ok(GoalFrame::Goal {
                    request_id,
                    binding,
                    goal: Some(goal),
                }),
                GoalResult::Missing => Ok(GoalFrame::Goal {
                    request_id,
                    binding,
                    goal: None,
                }),
                other => rejected(&other),
            }
        }
        GoalFrame::List {
            request_id,
            conversation_id,
        } => match service
            .list(&conversation_id.0)
            .map_err(|error| error.to_string())?
        {
            GoalResult::Goals(goals) => Ok(GoalFrame::Goals {
                request_id,
                conversation_id,
                goals,
            }),
            other => rejected(&other),
        },
        _ => Err("goal response frames are server-only".into()),
    }
}

fn context(binding: &gent_types::GoalBinding, host_epoch: HostEpoch) -> GoalControlContext {
    GoalControlContext {
        conversation_id: binding.conversation_id.0.clone(),
        run_id: binding.run_id.0.clone(),
        host_epoch,
    }
}

fn create_reply(request_id: String, result: GoalResult) -> Result<GoalFrame, String> {
    match result {
        GoalResult::Goal(goal) => Ok(GoalFrame::Created { request_id, goal }),
        other => rejected(&other),
    }
}

fn transition_reply(request_id: String, result: GoalResult) -> Result<GoalFrame, String> {
    match result {
        GoalResult::Goal(goal) => Ok(GoalFrame::Transitioned { request_id, goal }),
        other => rejected(&other),
    }
}

fn rejected(result: &GoalResult) -> Result<GoalFrame, String> {
    let message = match result {
        GoalResult::DeniedObserver => "goals are unavailable while gentd is observer-disabled",
        GoalResult::Missing => "goal is not present",
        GoalResult::Rejected(_) => "goal request was rejected",
        GoalResult::Goal(_) | GoalResult::Goals(_) => "goal request had an invalid result",
    };
    Err(message.into())
}
