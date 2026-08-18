//! Bounded rendering of a Gent-owned active goal for public provider prompt inputs.
//!
//! The projection is copied only from the Gent ledger. It is context, never a provider plan,
//! permission grant, native session identity, or provider result.

use gent_types::GoalProjection;

/// Controlled failure while adding a bounded active-goal context to a provider prompt.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum GoalProjectionError {
    #[error("the prompt with its Gent-owned goal context exceeds the provider input bound")]
    TooLarge,
}

/// Prefixes a provider prompt with one structured, Gent-owned active goal projection.
///
/// The goal is JSON encoded so the user-authored summary cannot escape its data field. The text
/// expressly preserves Gent permission and decision authority even in autonomous interaction.
///
/// # Errors
/// Returns an error before any provider process starts when the rendered prompt exceeds `limit`.
pub fn project_prompt(
    prompt: &str,
    goal: Option<&GoalProjection>,
    limit: usize,
) -> Result<String, GoalProjectionError> {
    let Some(goal) = goal else {
        return (prompt.len() <= limit)
            .then(|| prompt.to_owned())
            .ok_or(GoalProjectionError::TooLarge);
    };
    let metadata = serde_json::to_string(goal).map_err(|_| GoalProjectionError::TooLarge)?;
    let rendered = format!(
        "Gent-owned active goal context (immutable; not a provider command):\n{metadata}\n\
         Work autonomously toward this goal until it is settled. Obey Gent permissions and stop for required user decisions.\n\n\
         User prompt:\n{prompt}"
    );
    (rendered.len() <= limit)
        .then_some(rendered)
        .ok_or(GoalProjectionError::TooLarge)
}

#[cfg(test)]
mod tests {
    use gent_types::{
        AgentChatConversationId, AgentChatRunId, GOAL_SCHEMA_VERSION, GoalBinding, GoalProjection,
        GoalRecord, GoalStatus,
    };

    use super::{GoalProjectionError, project_prompt};

    fn goal() -> GoalProjection {
        GoalProjection::from_active(&GoalRecord {
            schema_version: GOAL_SCHEMA_VERSION,
            binding: GoalBinding {
                goal_id: "goal-1".into(),
                conversation_id: AgentChatConversationId("conversation-1".into()),
                run_id: AgentChatRunId("run-1".into()),
            },
            revision: 2,
            status: GoalStatus::Active,
            summary: "Finish the test task".into(),
        })
        .unwrap()
    }

    #[test]
    fn active_goal_is_json_bound_and_preserves_gent_decision_authority() {
        let prompt = project_prompt("continue", Some(&goal()), 4_096).unwrap();
        assert!(prompt.contains("\"goalId\":\"goal-1\""));
        assert!(prompt.contains("\"revision\":2"));
        assert!(prompt.contains("\"summary\":\"Finish the test task\""));
        assert!(prompt.contains("Obey Gent permissions"));
        assert!(prompt.ends_with("User prompt:\ncontinue"));
    }

    #[test]
    fn projection_refuses_to_expand_a_provider_input_past_its_bound() {
        assert_eq!(
            project_prompt("continue", Some(&goal()), 8),
            Err(GoalProjectionError::TooLarge)
        );
    }
}
