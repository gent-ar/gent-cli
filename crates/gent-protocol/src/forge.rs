use gent_types::ForgeConnectorRecord;
use serde::{Deserialize, Serialize};

pub const FORGE_CONNECTORS_CAPABILITY: &str = "forge-connectors-v1";
const MAX_CONNECTORS: usize = 128;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ForgeConnectorFrame {
    ListRequest {
        request_id: String,
        workspace_id: String,
    },
    List {
        request_id: String,
        workspace_id: String,
        connectors: Vec<ForgeConnectorRecord>,
    },
    GetRequest {
        request_id: String,
        workspace_id: String,
        connector_id: String,
    },
    Get {
        request_id: String,
        workspace_id: String,
        connector: Option<ForgeConnectorRecord>,
    },
    CreateRequest {
        request_id: String,
        connector: ForgeConnectorRecord,
    },
    Created {
        request_id: String,
        connector: ForgeConnectorRecord,
    },
    SetEnabledRequest {
        request_id: String,
        workspace_id: String,
        connector_id: String,
        enabled: bool,
    },
    SetEnabled {
        request_id: String,
        connector: ForgeConnectorRecord,
    },
    InvokeRequest {
        request_id: String,
        workspace_id: String,
        connector_id: String,
        tool_name: Option<String>,
    },
    InvocationHandoff {
        request_id: String,
        workspace_id: String,
        connector_id: String,
        tool_source_id: String,
        tool_name: Option<String>,
    },
}

impl ForgeConnectorFrame {
    pub fn validate(&self) -> Result<(), ForgeConnectorFrameError> {
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
                connectors,
            } => {
                valid_id(request_id)?;
                valid_id(workspace_id)?;
                validate_connectors(workspace_id, connectors)?;
            }
            Self::GetRequest {
                request_id,
                workspace_id,
                connector_id,
            } => {
                valid_id(request_id)?;
                valid_id(workspace_id)?;
                valid_id(connector_id)?;
            }
            Self::Get {
                request_id,
                workspace_id,
                connector,
            } => {
                valid_id(request_id)?;
                valid_id(workspace_id)?;
                if let Some(connector) = connector {
                    validate_connectors(workspace_id, std::slice::from_ref(connector))?;
                }
            }
            Self::CreateRequest {
                request_id,
                connector,
            } => {
                valid_id(request_id)?;
                validate_connector(connector)?;
            }
            Self::Created {
                request_id,
                connector,
            } => {
                valid_id(request_id)?;
                validate_connector(connector)?;
            }
            Self::SetEnabledRequest {
                request_id,
                workspace_id,
                connector_id,
                ..
            } => {
                valid_id(request_id)?;
                valid_id(workspace_id)?;
                valid_id(connector_id)?;
            }
            Self::SetEnabled {
                request_id,
                connector,
            } => {
                valid_id(request_id)?;
                validate_connector(connector)?;
            }
            Self::InvokeRequest {
                request_id,
                workspace_id,
                connector_id,
                tool_name,
            } => {
                valid_id(request_id)?;
                valid_id(workspace_id)?;
                valid_id(connector_id)?;
                if let Some(tool_name) = tool_name {
                    valid_id(tool_name)?;
                }
            }
            Self::InvocationHandoff {
                request_id,
                workspace_id,
                connector_id,
                tool_source_id,
                tool_name,
            } => {
                valid_id(request_id)?;
                valid_id(workspace_id)?;
                valid_id(connector_id)?;
                valid_id(tool_source_id)?;
                if let Some(tool_name) = tool_name {
                    valid_id(tool_name)?;
                }
            }
        }
        Ok(())
    }
}

fn validate_connectors(
    workspace_id: &str,
    connectors: &[ForgeConnectorRecord],
) -> Result<(), ForgeConnectorFrameError> {
    if connectors.len() > MAX_CONNECTORS {
        return Err(ForgeConnectorFrameError::TooMany);
    }
    if connectors
        .iter()
        .any(|item| item.workspace_id != workspace_id)
    {
        return Err(ForgeConnectorFrameError::WorkspaceMismatch);
    }
    connectors.iter().try_for_each(validate_connector)
}

fn validate_connector(connector: &ForgeConnectorRecord) -> Result<(), ForgeConnectorFrameError> {
    valid_id(&connector.connector_id)?;
    valid_id(&connector.workspace_id)?;
    valid_text(&connector.name, 160)?;
    valid_text(&connector.description, 2000)?;
    valid_text(&connector.category, 160)?;
    connector
        .declared_tools
        .iter()
        .chain(connector.discovered_tools.iter())
        .try_for_each(|tool| valid_id(tool))
}

fn valid_text(value: &str, limit: usize) -> Result<(), ForgeConnectorFrameError> {
    (!value.is_empty() && value.len() <= limit && !value.chars().any(char::is_control))
        .then_some(())
        .ok_or(ForgeConnectorFrameError::Invalid)
}

fn valid_id(value: &str) -> Result<(), ForgeConnectorFrameError> {
    (!value.is_empty()
        && value.len() <= 200
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/')
        }))
    .then_some(())
    .ok_or(ForgeConnectorFrameError::Invalid)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ForgeConnectorFrameError {
    #[error("Forge connector frame is invalid")]
    Invalid,
    #[error("Forge connector frame contains too many records")]
    TooMany,
    #[error("Forge connector belongs to another workspace")]
    WorkspaceMismatch,
}

#[cfg(test)]
mod tests {
    use super::{FORGE_CONNECTORS_CAPABILITY, ForgeConnectorFrame};

    #[test]
    fn catalog_contract_has_a_stable_capability_and_rejects_unknown_fields() {
        assert_eq!(FORGE_CONNECTORS_CAPABILITY, "forge-connectors-v1");
        assert!(serde_json::from_str::<ForgeConnectorFrame>(r#"{"type":"listRequest","body":{"requestId":"request","workspaceId":"workspace","extra":true}}"#).is_err());
    }
}
