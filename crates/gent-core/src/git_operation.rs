//! Pure monotonic lifecycle policy for durable Git operations.

use gent_types::GitOperationPhase;

/// Reports whether one Git operation phase may transition to the next phase.
#[must_use]
pub fn permits_git_operation_transition(
    current: GitOperationPhase,
    next: GitOperationPhase,
) -> bool {
    if current.is_terminal() {
        return current == next;
    }
    matches!(
        (current, next),
        (
            GitOperationPhase::Requested,
            GitOperationPhase::Requested
                | GitOperationPhase::Running
                | GitOperationPhase::Interrupted
        ) | (
            GitOperationPhase::Running,
            GitOperationPhase::Running
                | GitOperationPhase::Succeeded
                | GitOperationPhase::Failed
                | GitOperationPhase::Interrupted
        )
    )
}

#[cfg(test)]
mod tests {
    use gent_types::GitOperationPhase;

    use super::permits_git_operation_transition;

    #[test]
    fn operation_cannot_escape_a_terminal_phase() {
        assert!(permits_git_operation_transition(
            GitOperationPhase::Running,
            GitOperationPhase::Succeeded
        ));
        assert!(!permits_git_operation_transition(
            GitOperationPhase::Succeeded,
            GitOperationPhase::Running
        ));
        assert!(permits_git_operation_transition(
            GitOperationPhase::Interrupted,
            GitOperationPhase::Interrupted
        ));
    }
}
