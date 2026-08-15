use gent_ports::AutomationExecutionUpdate;
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AutomationExecutionPhase, AutomationExecutionRecord, CapabilitySet, WorkspaceRecord,
};

#[test]
fn coordinator_records_without_starting_automation_work() {
    let coordinator =
        Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
    coordinator
        .create_workspace(&WorkspaceRecord {
            workspace_id: "workspace-a".into(),
            canonical_path: "/workspace".into(),
        })
        .unwrap();
    let execution = AutomationExecutionRecord {
        execution_id: "execution-a".into(),
        workspace_id: "workspace-a".into(),
        automation_id: "nightly".into(),
        trigger_key: "manual:one".into(),
        phase: AutomationExecutionPhase::Queued,
    };
    coordinator.create_automation_execution(&execution).unwrap();
    assert!(matches!(
        coordinator
            .transition_automation_execution(
                "execution-a",
                AutomationExecutionPhase::Queued,
                AutomationExecutionPhase::Running,
            )
            .unwrap(),
        AutomationExecutionUpdate::Applied(_)
    ));
    assert_eq!(
        coordinator
            .automation_executions("workspace-a")
            .unwrap()
            .len(),
        1
    );
    assert!(
        coordinator
            .transition_automation_execution(
                "execution-a",
                AutomationExecutionPhase::Running,
                AutomationExecutionPhase::Queued,
            )
            .is_err()
    );
}
