//! Pure validation for durable, cursor-ordered conversation activity facts.

use gent_types::{ConversationActivityFact, ConversationActivityScope};

/// Rejects an activity fact that cannot be placed in an immutable run history.
///
/// # Errors
/// Returns a stable invariant message when identity or its assigned cursor is invalid.
pub fn validate_conversation_activity_fact(fact: &ConversationActivityFact) -> Result<(), String> {
    let scope = activity_scope(fact);
    if [&scope.conversation_id, &scope.run_id, &scope.turn_id]
        .iter()
        .any(|value| value.trim().is_empty())
        || scope.host_epoch.0 == 0
        || scope.cursor == 0
    {
        return Err("conversation activity fact identity is invalid".into());
    }
    Ok(())
}

/// Returns the common immutable scope carried by an activity fact.
#[must_use]
pub fn activity_scope(fact: &ConversationActivityFact) -> &ConversationActivityScope {
    fact.scope()
}

/// Assigns the durable source cursor after a provider fact has been accepted.
#[must_use]
pub fn with_activity_cursor(
    mut fact: ConversationActivityFact,
    cursor: u64,
) -> ConversationActivityFact {
    match &mut fact {
        ConversationActivityFact::TurnStarted { scope }
        | ConversationActivityFact::RootActivity { scope, .. }
        | ConversationActivityFact::RootPhase { scope, .. }
        | ConversationActivityFact::WorkPhase { scope, .. }
        | ConversationActivityFact::DecisionPending { scope, .. }
        | ConversationActivityFact::DecisionSettled { scope, .. }
        | ConversationActivityFact::InterruptRequested { scope }
        | ConversationActivityFact::Recovered { scope }
        | ConversationActivityFact::Terminal { scope, .. } => scope.cursor = cursor,
    }
    fact
}

#[cfg(test)]
mod tests {
    use gent_types::{ConversationActivityFact, ConversationActivityScope, HostEpoch};

    use super::{validate_conversation_activity_fact, with_activity_cursor};

    #[test]
    fn assigned_cursor_makes_a_fact_valid() {
        let fact = with_activity_cursor(
            ConversationActivityFact::TurnStarted {
                scope: ConversationActivityScope {
                    conversation_id: "conversation".into(),
                    run_id: "run".into(),
                    turn_id: "turn".into(),
                    host_epoch: HostEpoch(1),
                    cursor: 0,
                },
            },
            4,
        );
        assert!(validate_conversation_activity_fact(&fact).is_ok());
    }
}
