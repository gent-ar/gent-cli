//! Pure monotonic state policy for durable turns.

use gent_types::DurableTurnPhase;

/// Reports whether a durable turn may move between two lifecycle phases.
///
/// Terminal state never changes; this prevents a late provider event from reviving a settled turn.
#[must_use]
pub fn permits_turn_transition(current: DurableTurnPhase, next: DurableTurnPhase) -> bool {
    if current.is_terminal() {
        return current == next;
    }
    matches!(
        (current, next),
        (DurableTurnPhase::Active, _)
            | (
                DurableTurnPhase::WaitingPermission | DurableTurnPhase::WaitingQuestion,
                DurableTurnPhase::Active
            )
            | (
                DurableTurnPhase::WaitingPermission,
                DurableTurnPhase::WaitingPermission
            )
            | (
                DurableTurnPhase::WaitingQuestion,
                DurableTurnPhase::WaitingQuestion
            )
    )
}

#[cfg(test)]
mod tests {
    use gent_types::DurableTurnPhase;

    use super::permits_turn_transition;

    #[test]
    fn terminal_turns_cannot_be_revived() {
        assert!(!permits_turn_transition(
            DurableTurnPhase::Completed,
            DurableTurnPhase::Active
        ));
        assert!(permits_turn_transition(
            DurableTurnPhase::Completed,
            DurableTurnPhase::Completed
        ));
    }

    #[test]
    fn waiting_turns_only_resume_or_repeat() {
        assert!(permits_turn_transition(
            DurableTurnPhase::WaitingQuestion,
            DurableTurnPhase::Active
        ));
        assert!(!permits_turn_transition(
            DurableTurnPhase::WaitingQuestion,
            DurableTurnPhase::WaitingPermission
        ));
    }
}
