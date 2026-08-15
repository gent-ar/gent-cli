use gent_ports::{AutomationExecutionLedger, AutomationExecutionUpdate, WorkspaceLedger};
use gent_store::SqliteLedger;
use gent_types::{AutomationExecutionPhase, AutomationExecutionRecord, WorkspaceRecord};

fn workspace() -> WorkspaceRecord {
    WorkspaceRecord {
        workspace_id: "workspace-a".into(),
        canonical_path: "/workspace".into(),
    }
}

fn execution(phase: AutomationExecutionPhase) -> AutomationExecutionRecord {
    AutomationExecutionRecord {
        execution_id: "execution-a".into(),
        workspace_id: "workspace-a".into(),
        automation_id: "nightly".into(),
        trigger_key: "cron:2026-08-15T00:00:00Z".into(),
        phase,
    }
}

#[test]
fn executions_are_durable_and_optimistic() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    ledger.create_workspace(&workspace()).unwrap();
    ledger
        .create_automation_execution(&execution(AutomationExecutionPhase::Queued))
        .unwrap();
    assert_eq!(
        ledger.find_automation_execution("execution-a").unwrap(),
        Some(execution(AutomationExecutionPhase::Queued))
    );
    assert!(matches!(
        ledger
            .replace_automation_execution_phase(
                "execution-a",
                AutomationExecutionPhase::Queued,
                AutomationExecutionPhase::Running,
            )
            .unwrap(),
        AutomationExecutionUpdate::Applied(AutomationExecutionRecord {
            phase: AutomationExecutionPhase::Running,
            ..
        })
    ));
    assert!(matches!(
        ledger
            .replace_automation_execution_phase(
                "execution-a",
                AutomationExecutionPhase::Queued,
                AutomationExecutionPhase::Interrupted,
            )
            .unwrap(),
        AutomationExecutionUpdate::Current(_)
    ));
    drop(ledger);
    assert_eq!(
        SqliteLedger::open(path)
            .unwrap()
            .list_automation_executions("workspace-a")
            .unwrap(),
        vec![execution(AutomationExecutionPhase::Running)]
    );
}

#[test]
fn executions_round_trip_every_terminal_phase() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger.create_workspace(&workspace()).unwrap();
    for (id, phase) in [
        ("succeeded", AutomationExecutionPhase::Succeeded),
        ("failed", AutomationExecutionPhase::Failed),
        ("interrupted", AutomationExecutionPhase::Interrupted),
    ] {
        let record = AutomationExecutionRecord {
            execution_id: id.into(),
            trigger_key: format!("manual:{id}"),
            ..execution(AutomationExecutionPhase::Queued)
        };
        ledger.create_automation_execution(&record).unwrap();
        ledger
            .replace_automation_execution_phase(
                id,
                AutomationExecutionPhase::Queued,
                AutomationExecutionPhase::Running,
            )
            .unwrap();
        ledger
            .replace_automation_execution_phase(id, AutomationExecutionPhase::Running, phase)
            .unwrap();
    }
    let phases = ledger
        .list_automation_executions("workspace-a")
        .unwrap()
        .into_iter()
        .map(|record| record.phase)
        .collect::<Vec<_>>();
    assert_eq!(
        phases,
        vec![
            AutomationExecutionPhase::Succeeded,
            AutomationExecutionPhase::Failed,
            AutomationExecutionPhase::Interrupted,
        ]
    );
}

#[test]
fn executions_require_a_workspace_unique_trigger_and_queued_phase() {
    let ledger = SqliteLedger::in_memory().unwrap();
    assert!(
        ledger
            .create_automation_execution(&execution(AutomationExecutionPhase::Queued))
            .is_err()
    );
    ledger.create_workspace(&workspace()).unwrap();
    assert!(
        ledger
            .create_automation_execution(&execution(AutomationExecutionPhase::Running))
            .is_err()
    );
    ledger
        .create_automation_execution(&execution(AutomationExecutionPhase::Queued))
        .unwrap();
    assert!(
        ledger
            .create_automation_execution(&AutomationExecutionRecord {
                execution_id: "execution-b".into(),
                ..execution(AutomationExecutionPhase::Queued)
            })
            .is_err()
    );
    assert!(
        ledger
            .create_automation_execution(&AutomationExecutionRecord {
                trigger_key: "bad key".into(),
                ..execution(AutomationExecutionPhase::Queued)
            })
            .is_err()
    );
}
