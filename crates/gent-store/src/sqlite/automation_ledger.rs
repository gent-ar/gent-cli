use gent_ports::{AutomationLedger, LedgerError};
use gent_types::{
    AutomationDefinition, AutomationId, AutomationRun, AutomationRunId, AutomationRunStatus,
    AutomationRunSummary,
};
use rusqlite::{OptionalExtension, params};

use super::{SqliteLedger, queries::storage_error};

impl AutomationLedger for SqliteLedger {
    fn create_automation(&self, definition: &AutomationDefinition) -> Result<(), LedgerError> {
        definition.validate().map_err(invariant)?;
        let action = serde_json::to_string(&definition.action).map_err(storage)?;
        let trigger = serde_json::to_string(&definition.trigger).map_err(storage)?;
        let notifications = serde_json::to_string(&definition.notifications).map_err(storage)?;
        self.lock()?.execute(
            "INSERT INTO automation_definitions (automation_id, workspace_id, name, working_directory, enabled, action, trigger, condition, selection, chain_target, notifications, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![definition.automation_id.0, definition.workspace_id, definition.name, definition.working_directory, definition.enabled, action, trigger, definition.condition, serde_json::to_string(&definition.selection).map_err(storage)?, definition.chain_target.as_ref().map(|id| &id.0), notifications, definition.created_at, definition.updated_at],
        ).map(|_| ()).map_err(storage_error)
    }

    fn find_automation(
        &self,
        id: &AutomationId,
    ) -> Result<Option<AutomationDefinition>, LedgerError> {
        let connection = self.lock()?;
        let definition = connection.query_row("SELECT automation_id, workspace_id, name, working_directory, enabled, action, trigger, condition, selection, chain_target, notifications, created_at, updated_at FROM automation_definitions WHERE automation_id = ?1", [&id.0], decode_definition).optional().map_err(storage_error)?;
        drop(connection);
        definition.map_or(Ok(None), |definition| {
            self.with_last_run(definition).map(Some)
        })
    }

    fn list_automations(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<AutomationDefinition>, LedgerError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare("SELECT automation_id, workspace_id, name, working_directory, enabled, action, trigger, condition, selection, chain_target, notifications, created_at, updated_at FROM automation_definitions WHERE workspace_id = ?1 ORDER BY rowid").map_err(storage_error)?;
        let definitions = statement
            .query_map([workspace_id], decode_definition)
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)?;
        drop(statement);
        drop(connection);
        definitions
            .into_iter()
            .map(|definition| self.with_last_run(definition))
            .collect()
    }

    fn record_automation_run(&self, run: &AutomationRun) -> Result<(), LedgerError> {
        run.validate().map_err(invariant)?;
        let connection = self.lock()?;
        if connection
            .query_row(
                "SELECT 1 FROM automation_definitions WHERE automation_id = ?1",
                [&run.automation_id.0],
                |_| Ok(()),
            )
            .optional()
            .map_err(storage_error)?
            .is_none()
        {
            return Err(invariant("automation definition does not exist"));
        }
        connection.execute("INSERT INTO automation_runs (run_id, automation_id, conversation_id, parent_run_id, started_at, ended_at, status, summary, error, condition_result) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)", params![run.run_id.0, run.automation_id.0, run.conversation_id, run.parent_run_id.as_ref().map(|id| &id.0), run.started_at, run.ended_at, encode_status(run.status), run.summary, run.error, run.condition_result]).map(|_| ()).map_err(storage_error)
    }

    fn find_automation_run(
        &self,
        id: &AutomationRunId,
    ) -> Result<Option<AutomationRun>, LedgerError> {
        self.lock()?.query_row("SELECT run_id, automation_id, conversation_id, parent_run_id, started_at, ended_at, status, summary, error, condition_result FROM automation_runs WHERE run_id = ?1", [&id.0], decode_run).optional().map_err(storage_error)
    }

    fn list_automation_runs(
        &self,
        id: &AutomationId,
        limit: u16,
    ) -> Result<Vec<AutomationRun>, LedgerError> {
        let limit = i64::from(limit.clamp(1, 100));
        let connection = self.lock()?;
        let mut statement = connection.prepare("SELECT run_id, automation_id, conversation_id, parent_run_id, started_at, ended_at, status, summary, error, condition_result FROM automation_runs WHERE automation_id = ?1 ORDER BY started_at DESC LIMIT ?2").map_err(storage_error)?;
        statement
            .query_map(params![id.0, limit], decode_run)
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)
    }
}

impl SqliteLedger {
    fn with_last_run(
        &self,
        mut definition: AutomationDefinition,
    ) -> Result<AutomationDefinition, LedgerError> {
        definition.last_run = self
            .list_automation_runs(&definition.automation_id, 1)?
            .into_iter()
            .next()
            .map(|run| AutomationRunSummary {
                run_id: run.run_id,
                status: run.status,
                started_at: run.started_at,
                ended_at: run.ended_at,
            });
        Ok(definition)
    }
}

fn decode_definition(row: &rusqlite::Row<'_>) -> rusqlite::Result<AutomationDefinition> {
    Ok(AutomationDefinition {
        automation_id: AutomationId(row.get(0)?),
        workspace_id: row.get(1)?,
        name: row.get(2)?,
        working_directory: row.get(3)?,
        enabled: row.get(4)?,
        action: from_json(row.get(5)?)?,
        trigger: from_json(row.get(6)?)?,
        condition: row.get(7)?,
        selection: from_json(row.get(8)?)?,
        chain_target: row.get::<_, Option<String>>(9)?.map(AutomationId),
        notifications: from_json(row.get(10)?)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
        last_run: None,
    })
}

fn decode_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<AutomationRun> {
    Ok(AutomationRun {
        run_id: AutomationRunId(row.get(0)?),
        automation_id: AutomationId(row.get(1)?),
        conversation_id: row.get(2)?,
        parent_run_id: row.get::<_, Option<String>>(3)?.map(AutomationRunId),
        started_at: row.get(4)?,
        ended_at: row.get(5)?,
        status: decode_status(&row.get::<_, String>(6)?)?,
        summary: row.get(7)?,
        error: row.get(8)?,
        condition_result: row.get(9)?,
    })
}

fn from_json<T: serde::de::DeserializeOwned>(value: String) -> rusqlite::Result<T> {
    serde_json::from_str(&value).map_err(|_| rusqlite::Error::InvalidQuery)
}
fn encode_status(status: AutomationRunStatus) -> &'static str {
    match status {
        AutomationRunStatus::Running => "running",
        AutomationRunStatus::Success => "success",
        AutomationRunStatus::Error => "error",
        AutomationRunStatus::Cancelled => "cancelled",
        AutomationRunStatus::Skipped => "skipped",
    }
}
fn decode_status(value: &str) -> rusqlite::Result<AutomationRunStatus> {
    match value {
        "running" => Ok(AutomationRunStatus::Running),
        "success" => Ok(AutomationRunStatus::Success),
        "error" => Ok(AutomationRunStatus::Error),
        "cancelled" => Ok(AutomationRunStatus::Cancelled),
        "skipped" => Ok(AutomationRunStatus::Skipped),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
fn invariant(value: &'static str) -> LedgerError {
    LedgerError::Invariant(value.into())
}
fn storage(error: serde_json::Error) -> LedgerError {
    LedgerError::Storage(error.to_string())
}

#[cfg(test)]
mod tests {
    use gent_ports::{AutomationLedger, WorkspaceLedger};
    use gent_types::{
        AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, AutomationAction,
        AutomationDefinition, AutomationId, AutomationNotifications, AutomationRun,
        AutomationRunId, AutomationRunStatus, AutomationTrigger, WorkspaceRecord,
    };

    use super::SqliteLedger;

    fn definition() -> AutomationDefinition {
        AutomationDefinition {
            automation_id: AutomationId("automation-1".into()),
            workspace_id: "workspace-1".into(),
            name: "Daily check".into(),
            working_directory: "/tmp/project".into(),
            enabled: true,
            action: AutomationAction::Prompt {
                prompt: "Check the project".into(),
            },
            trigger: AutomationTrigger::Manual,
            condition: None,
            selection: AgentChatSelection {
                provider: AgentChatProvider::Claude,
                model: "haiku".into(),
                effort: AgentChatEffort::Medium,
                mode: AgentChatMode::Agent,
            },
            chain_target: None,
            notifications: AutomationNotifications {
                on_success: true,
                on_error: true,
            },
            created_at: 1,
            updated_at: 1,
            last_run: None,
        }
    }

    #[test]
    fn persists_definition_and_run_projection() {
        let ledger = SqliteLedger::in_memory().unwrap();
        ledger
            .create_workspace(&WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/tmp/project".into(),
            })
            .unwrap();
        ledger.create_automation(&definition()).unwrap();
        ledger
            .record_automation_run(&AutomationRun {
                run_id: AutomationRunId("run-1".into()),
                automation_id: AutomationId("automation-1".into()),
                conversation_id: None,
                parent_run_id: None,
                started_at: 2,
                ended_at: Some(3),
                status: AutomationRunStatus::Success,
                summary: Some("done".into()),
                error: None,
                condition_result: Some(true),
            })
            .unwrap();
        let saved = ledger
            .find_automation(&AutomationId("automation-1".into()))
            .unwrap()
            .unwrap();
        assert_eq!(saved.last_run.unwrap().status, AutomationRunStatus::Success);
        assert_eq!(
            ledger
                .list_automation_runs(&saved.automation_id, 1)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn rejects_non_manual_trigger() {
        let mut value = definition();
        value.trigger = AutomationTrigger::Schedule {
            expression: "* * * * *".into(),
        };
        assert!(value.validate().is_err());
    }
}
