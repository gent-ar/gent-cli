use serde::{Deserialize, Serialize};

use crate::AgentChatSelection;

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct AutomationId(pub String);

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct AutomationRunId(pub String);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutomationDefinition {
    pub automation_id: AutomationId,
    pub workspace_id: String,
    pub name: String,
    pub working_directory: String,
    pub enabled: bool,
    pub action: AutomationAction,
    pub trigger: AutomationTrigger,
    pub condition: Option<String>,
    pub selection: AgentChatSelection,
    pub chain_target: Option<AutomationId>,
    pub notifications: AutomationNotifications,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_run: Option<AutomationRunSummary>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum AutomationAction {
    Prompt { prompt: String },
    Skill { skill: String },
    SkillAndPrompt { skill: String, prompt: String },
    Script { command: String, args: Vec<String> },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum AutomationTrigger {
    Manual,
    Schedule {
        expression: String,
    },
    Webhook {
        secret_name: String,
    },
    FileWatch {
        paths: Vec<String>,
        debounce_ms: u64,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutomationNotifications {
    pub on_success: bool,
    pub on_error: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutomationRun {
    pub run_id: AutomationRunId,
    pub automation_id: AutomationId,
    pub conversation_id: Option<String>,
    pub parent_run_id: Option<AutomationRunId>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub status: AutomationRunStatus,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub condition_result: Option<bool>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AutomationRunStatus {
    Running,
    Success,
    Error,
    Cancelled,
    Skipped,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutomationRunSummary {
    pub run_id: AutomationRunId,
    pub status: AutomationRunStatus,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

impl AutomationDefinition {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !valid_id(&self.automation_id.0)
            || !valid_id(&self.workspace_id)
            || !valid_text(&self.name, 256)
            || !valid_text(&self.working_directory, 4096)
            || self.updated_at < self.created_at
        {
            return Err("automation definition has invalid identity or metadata");
        }
        self.selection
            .validate()
            .map_err(|_| "automation selection is invalid")?;
        if self
            .condition
            .as_deref()
            .is_some_and(|condition| !valid_text(condition, 4096))
            || self
                .chain_target
                .as_ref()
                .is_some_and(|target| !valid_id(&target.0))
        {
            return Err("automation definition has invalid condition or chain target");
        }
        if !matches!(self.trigger, AutomationTrigger::Manual) {
            return Err("only manual automation triggers are available");
        }
        validate_action(&self.action)
    }
}

impl AutomationRun {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !valid_id(&self.run_id.0)
            || !valid_id(&self.automation_id.0)
            || self.ended_at.is_some_and(|ended| ended < self.started_at)
        {
            return Err("automation run has invalid identity or timestamps");
        }
        Ok(())
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn validate_action(action: &AutomationAction) -> Result<(), &'static str> {
    let valid = match action {
        AutomationAction::Prompt { prompt } => valid_text(prompt, 64_000),
        AutomationAction::Skill { skill } => valid_text(skill, 256),
        AutomationAction::SkillAndPrompt { skill, prompt } => {
            valid_text(skill, 256) && valid_text(prompt, 64_000)
        }
        AutomationAction::Script { command, args } => {
            valid_text(command, 4096)
                && args.len() <= 64
                && args.iter().all(|arg| valid_text(arg, 4096))
        }
    };
    valid
        .then_some(())
        .ok_or("automation action cannot be empty")
}

fn valid_text(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max_bytes && !value.contains('\0')
}
