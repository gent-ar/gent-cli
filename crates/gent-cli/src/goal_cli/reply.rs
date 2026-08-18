//! Exact request/reply correlation for the finite goal endpoint.

use gent_protocol::GoalFrame;
use gent_types::GoalStatus;

use super::StatusArgument;

pub(super) fn valid_reply(request: &GoalFrame, response: &GoalFrame) -> bool {
    match (request, response) {
        (
            GoalFrame::Create { request_id, goal },
            GoalFrame::Created {
                request_id: reply,
                goal: saved,
            },
        ) => reply == request_id && saved == goal,
        (
            GoalFrame::Read {
                request_id,
                binding,
            },
            GoalFrame::Goal {
                request_id: reply,
                binding: found,
                ..
            },
        ) => reply == request_id && found == binding,
        (
            GoalFrame::List {
                request_id,
                conversation_id,
            },
            GoalFrame::Goals {
                request_id: reply,
                conversation_id: found,
                ..
            },
        ) => reply == request_id && found == conversation_id,
        (
            GoalFrame::Transition {
                request_id,
                transition,
            },
            GoalFrame::Transitioned {
                request_id: reply,
                goal,
            },
        ) => {
            reply == request_id
                && goal.binding == transition.binding
                && goal.status == transition.next_status
                && goal.revision == transition.expected_revision.saturating_add(1)
        }
        _ => false,
    }
}

impl From<StatusArgument> for GoalStatus {
    fn from(value: StatusArgument) -> Self {
        match value {
            StatusArgument::Completed => Self::Completed,
            StatusArgument::Abandoned => Self::Abandoned,
            StatusArgument::Failed => Self::Failed,
        }
    }
}
