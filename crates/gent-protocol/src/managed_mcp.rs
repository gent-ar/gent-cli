use gent_types::{ForgeConnectorRecord, HostEpoch, ReceiptId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[path = "managed_mcp_secret.rs"]
mod secret;
pub use secret::CredentialSubmission;

pub const MANAGED_MCP_CAPABILITY: &str = "managed-mcp-registration-v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedMcpContext {
    pub host_epoch: HostEpoch,
    pub conversation_id: String,
    pub run_id: String,
    pub workspace_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CredentialField {
    pub field_id: String,
    pub label: String,
    pub environment_name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedMcpRegistration {
    pub connector_id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub project_directory: String,
    pub entry_path: String,
    pub declared_tools: Vec<String>,
    pub credential_fields: Vec<CredentialField>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ManagedMcpState {
    Prepared,
    CredentialsNeeded,
    CredentialStorageUnavailable,
    PendingPermission,
    DeniedByPolicy,
    DeniedByPlan,
    ContainmentUnavailable,
    Connected,
    Failed,
    Rejected,
    Unprovable,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ManagedMcpFrame {
    ManagedMcpRegister {
        request_id: String,
        receipt_id: ReceiptId,
        context: ManagedMcpContext,
        registration: ManagedMcpRegistration,
    },
    ManagedMcpSubmitCredentials {
        request_id: String,
        receipt_id: ReceiptId,
        context: ManagedMcpContext,
        connector_id: String,
        credentials: CredentialSubmission,
    },
    ManagedMcpConnect {
        request_id: String,
        receipt_id: ReceiptId,
        context: ManagedMcpContext,
        connector_id: String,
    },
    ManagedMcpCatalog {
        request_id: String,
        context: ManagedMcpContext,
        connector_id: String,
    },
    ManagedMcpCall {
        request_id: String,
        receipt_id: ReceiptId,
        context: ManagedMcpContext,
        connector_id: String,
        tool_name: String,
        arguments: Value,
    },
    ManagedMcpResult {
        request_id: String,
        state: ManagedMcpState,
        connector: Option<ForgeConnectorRecord>,
        credential_fields: Vec<CredentialField>,
    },
    ManagedMcpTools {
        request_id: String,
        tools: Vec<Value>,
    },
    ManagedMcpToolResult {
        request_id: String,
        result: Value,
    },
}

impl ManagedMcpFrame {
    pub fn is_request(&self) -> bool {
        !matches!(
            self,
            Self::ManagedMcpResult { .. }
                | Self::ManagedMcpTools { .. }
                | Self::ManagedMcpToolResult { .. }
        )
    }

    pub fn validate(&self) -> bool {
        match self {
            Self::ManagedMcpRegister {
                request_id,
                receipt_id,
                context,
                registration,
            } => {
                valid_id(request_id)
                    && valid_id(&receipt_id.0)
                    && context.validate()
                    && registration.validate()
            }
            Self::ManagedMcpSubmitCredentials {
                request_id,
                receipt_id,
                context,
                connector_id,
                credentials,
            } => {
                valid_id(request_id)
                    && valid_id(&receipt_id.0)
                    && context.validate()
                    && valid_id(connector_id)
                    && credentials.validate()
            }
            Self::ManagedMcpConnect {
                request_id,
                receipt_id,
                context,
                connector_id,
            } => {
                valid_id(request_id)
                    && valid_id(&receipt_id.0)
                    && context.validate()
                    && valid_id(connector_id)
            }
            Self::ManagedMcpCatalog {
                request_id,
                context,
                connector_id,
            } => valid_id(request_id) && context.validate() && valid_id(connector_id),
            Self::ManagedMcpCall {
                request_id,
                receipt_id,
                context,
                connector_id,
                tool_name,
                arguments,
            } => {
                valid_id(request_id)
                    && valid_id(&receipt_id.0)
                    && context.validate()
                    && valid_id(connector_id)
                    && valid_id(tool_name)
                    && arguments.is_object()
                    && serde_json::to_vec(arguments).is_ok_and(|value| value.len() <= 64 * 1024)
            }
            Self::ManagedMcpResult {
                request_id,
                credential_fields,
                ..
            } => {
                valid_id(request_id)
                    && credential_fields.len() <= 16
                    && credential_fields.iter().all(CredentialField::validate)
            }
            Self::ManagedMcpTools { request_id, tools } => {
                valid_id(request_id) && tools.len() <= 128
            }
            Self::ManagedMcpToolResult { request_id, result } => {
                valid_id(request_id)
                    && serde_json::to_vec(result).is_ok_and(|value| value.len() <= 128 * 1024)
            }
        }
    }
}

impl ManagedMcpContext {
    pub fn validate(&self) -> bool {
        self.host_epoch.0 > 0
            && [&self.conversation_id, &self.run_id, &self.workspace_id]
                .into_iter()
                .all(|value| valid_id(value))
    }
}

impl CredentialField {
    pub fn validate(&self) -> bool {
        valid_credential_field_id(&self.field_id)
            && valid_text(&self.label, 160)
            && !self.environment_name.is_empty()
            && self.environment_name.len() <= 128
            && self
                .environment_name
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
            && !matches!(
                self.environment_name.as_str(),
                "PATH"
                    | "HOME"
                    | "USERPROFILE"
                    | "SYSTEMROOT"
                    | "NODE_OPTIONS"
                    | "NODE_PATH"
                    | "LD_PRELOAD"
                    | "LD_LIBRARY_PATH"
            )
            && !self.environment_name.starts_with("DYLD_")
            && !self.environment_name.starts_with("GENT_")
    }
}

impl ManagedMcpRegistration {
    pub fn validate(&self) -> bool {
        valid_id(&self.connector_id)
            && valid_text(&self.name, 160)
            && valid_text(&self.description, 2000)
            && valid_text(&self.category, 160)
            && valid_text(&self.project_directory, 32768)
            && valid_text(&self.entry_path, 32768)
            && !self.declared_tools.is_empty()
            && self.declared_tools.len() <= 128
            && self.declared_tools.iter().all(|value| valid_id(value))
            && self.declared_tools.windows(2).all(|pair| pair[0] < pair[1])
            && self.credential_fields.len() <= 16
            && self.credential_fields.iter().all(CredentialField::validate)
            && self
                .credential_fields
                .iter()
                .enumerate()
                .all(|(index, value)| {
                    self.credential_fields[..index].iter().all(|other| {
                        value.field_id != other.field_id
                            && value.environment_name != other.environment_name
                    })
                })
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/')
        })
}

fn valid_credential_field_id(value: &str) -> bool {
    value.len() <= 128 && valid_id(value)
}

fn valid_text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}
