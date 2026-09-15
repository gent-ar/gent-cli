//! Exact request/reply correlation for the finite goal endpoint.

use gent_protocol::GoalFrame;

pub(super) fn valid_reply(request: &GoalFrame, response: &GoalFrame) -> bool {
    if request.request_id() != response.request_id() {
        return false;
    }
    match (request, response) {
        (_, GoalFrame::Rejected { .. }) => true,
        (GoalFrame::Report { goal_id, .. }, GoalFrame::Goal { goal, .. }) => goal
            .as_ref()
            .is_some_and(|goal| goal.binding.goal_id == *goal_id),
        (
            GoalFrame::Set {
                conversation_id, ..
            }
            | GoalFrame::Pause {
                conversation_id, ..
            }
            | GoalFrame::Resume {
                conversation_id, ..
            }
            | GoalFrame::Clear {
                conversation_id, ..
            }
            | GoalFrame::Read {
                conversation_id, ..
            },
            GoalFrame::Goal {
                conversation_id: reply,
                ..
            },
        ) => reply == conversation_id,
        _ => false,
    }
}
