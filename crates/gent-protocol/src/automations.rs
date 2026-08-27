use gent_types::{AutomationDefinition, AutomationId, AutomationRun, AutomationRunId};
use serde::{Deserialize, Serialize};

pub const AUTOMATIONS_CAPABILITY: &str = "automations-v1";
const MAX_AUTOMATIONS: usize = 128;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AutomationFrame {
    ListRequest {
        request_id: String,
        workspace_id: String,
    },
    List {
        request_id: String,
        workspace_id: String,
        automations: Vec<AutomationDefinition>,
    },
    CreateRequest {
        request_id: String,
        definition: AutomationDefinition,
    },
    Created {
        request_id: String,
        definition: AutomationDefinition,
    },
    RunRequest {
        request_id: String,
        automation_id: AutomationId,
    },
    RunAccepted {
        request_id: String,
        automation_id: AutomationId,
        run_id: AutomationRunId,
        conversation_id: String,
        agent_chat_run_id: String,
        turn_id: String,
    },
    RunsRequest {
        request_id: String,
        automation_id: AutomationId,
        limit: u16,
    },
    Runs {
        request_id: String,
        automation_id: AutomationId,
        runs: Vec<AutomationRun>,
    },
}

impl AutomationFrame {
    pub fn validate(&self) -> Result<(), AutomationFrameError> {
        match self {
            Self::ListRequest {
                request_id,
                workspace_id,
            } => {
                valid_id(request_id)?;
                valid_id(workspace_id)?;
            }
            Self::List {
                request_id,
                workspace_id,
                automations,
            } => {
                valid_id(request_id)?;
                valid_id(workspace_id)?;
                if automations.len() > MAX_AUTOMATIONS {
                    return Err(AutomationFrameError::TooMany);
                }
                for automation in automations {
                    automation
                        .validate()
                        .map_err(|_| AutomationFrameError::Invalid)?;
                    if automation.workspace_id != *workspace_id {
                        return Err(AutomationFrameError::WorkspaceMismatch);
                    }
                }
            }
            Self::CreateRequest {
                request_id,
                definition,
            }
            | Self::Created {
                request_id,
                definition,
            } => {
                valid_id(request_id)?;
                definition
                    .validate()
                    .map_err(|_| AutomationFrameError::Invalid)?;
            }
            Self::RunRequest {
                request_id,
                automation_id,
            } => {
                valid_id(request_id)?;
                valid_id(&automation_id.0)?;
            }
            Self::RunAccepted {
                request_id,
                automation_id,
                run_id,
                conversation_id,
                agent_chat_run_id,
                turn_id,
            } => {
                for value in [
                    request_id,
                    &automation_id.0,
                    &run_id.0,
                    conversation_id,
                    agent_chat_run_id,
                    turn_id,
                ] {
                    valid_id(value)?;
                }
            }
            Self::RunsRequest {
                request_id,
                automation_id,
                limit,
            } => {
                valid_id(request_id)?;
                valid_id(&automation_id.0)?;
                if *limit == 0 || *limit > 100 {
                    return Err(AutomationFrameError::Invalid);
                }
            }
            Self::Runs {
                request_id,
                automation_id,
                runs,
            } => {
                valid_id(request_id)?;
                valid_id(&automation_id.0)?;
                if runs.len() > MAX_AUTOMATIONS {
                    return Err(AutomationFrameError::TooMany);
                }
                for run in runs {
                    run.validate().map_err(|_| AutomationFrameError::Invalid)?;
                    if run.automation_id != *automation_id {
                        return Err(AutomationFrameError::WorkspaceMismatch);
                    }
                }
            }
        }
        Ok(())
    }
}

fn valid_id(value: &str) -> Result<(), AutomationFrameError> {
    (!value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')))
    .then_some(())
    .ok_or(AutomationFrameError::Invalid)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AutomationFrameError {
    #[error("automation frame is invalid")]
    Invalid,
    #[error("automation frame contains too many records")]
    TooMany,
    #[error("automation belongs to another workspace")]
    WorkspaceMismatch,
}

#[cfg(test)]
mod tests {
    use super::{AUTOMATIONS_CAPABILITY, AutomationFrame};

    #[test]
    fn catalog_contract_has_a_stable_capability_and_rejects_unknown_fields() {
        assert_eq!(AUTOMATIONS_CAPABILITY, "automations-v1");
        assert!(serde_json::from_str::<AutomationFrame>(r#"{"type":"listRequest","body":{"requestId":"request","workspaceId":"workspace","extra":true}}"#).is_err());
    }
}
