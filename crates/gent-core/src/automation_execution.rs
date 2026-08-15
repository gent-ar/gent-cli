//! Pure monotonic lifecycle policy for durable automation executions.

use gent_types::AutomationExecutionPhase;

/// Reports whether an automation execution phase may transition to the next phase.
#[must_use]
pub fn permits_automation_execution_transition(
    current: AutomationExecutionPhase,
    next: AutomationExecutionPhase,
) -> bool {
    if current.is_terminal() {
        return current == next;
    }
    matches!(
        (current, next),
        (
            AutomationExecutionPhase::Queued,
            AutomationExecutionPhase::Queued
                | AutomationExecutionPhase::Running
                | AutomationExecutionPhase::Interrupted
        ) | (
            AutomationExecutionPhase::Running,
            AutomationExecutionPhase::Running
                | AutomationExecutionPhase::Succeeded
                | AutomationExecutionPhase::Failed
                | AutomationExecutionPhase::Interrupted
        )
    )
}

#[cfg(test)]
mod tests {
    use gent_types::AutomationExecutionPhase;

    use super::permits_automation_execution_transition;

    #[test]
    fn execution_cannot_escape_a_terminal_phase() {
        assert!(permits_automation_execution_transition(
            AutomationExecutionPhase::Queued,
            AutomationExecutionPhase::Interrupted
        ));
        assert!(permits_automation_execution_transition(
            AutomationExecutionPhase::Running,
            AutomationExecutionPhase::Succeeded
        ));
        assert!(!permits_automation_execution_transition(
            AutomationExecutionPhase::Failed,
            AutomationExecutionPhase::Running
        ));
    }
}
