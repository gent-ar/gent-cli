use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const ACTIVATION_OPERATION_CAPABILITY: &str = "runtime-activation-operation-v1";
pub const ACTIVATION_MAINTENANCE_MAX_BYTES: usize = 128 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpectedActivationOwner {
    pub process_id: u32,
    pub executable_digest_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationOperationExpectation {
    pub operation_id: String,
    pub expected_owner: ExpectedActivationOwner,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationCommand {
    pub bootstrap_directory: PathBuf,
    pub runtime_root: PathBuf,
    pub data_directory: PathBuf,
    pub expected_daemon_sha256: Option<String>,
    pub expected_bootstrap_tree_sha256: Option<String>,
    pub expected_host_epoch: Option<u64>,
    pub operation: Option<ActivationOperationExpectation>,
    pub expected_command_sha256: Option<String>,
    pub maintenance_daemon_sha256: String,
    pub enable_automatic_updates: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationLookup {
    pub operation_id: String,
    pub command_sha256: String,
    pub runtime_root: PathBuf,
    pub data_directory: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationOriginLookup {
    pub runtime_root: PathBuf,
    pub data_directory: PathBuf,
    pub expected_host_epoch: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ActivationOriginState {
    Closed,
    Busy,
    Unavailable,
    NotClosed,
    EpochMismatch,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationOriginStatus {
    pub schema: u16,
    pub runtime_root: PathBuf,
    pub data_directory: PathBuf,
    pub state: ActivationOriginState,
    pub observed_host_epoch: Option<u64>,
    pub observed_closed_ingress: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(
    tag = "action",
    content = "body",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum ActivationMaintenanceAction {
    Capabilities,
    BootstrapIdentity { bootstrap_directory: PathBuf },
    Activate(ActivationCommand),
    Plan(ActivationCommand),
    OriginStatus(ActivationOriginLookup),
    Status(ActivationLookup),
    Adopt(ActivationLookup),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationMaintenanceRequest {
    pub schema: u16,
    pub expected_maintenance_daemon_sha256: String,
    pub action: ActivationMaintenanceAction,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationResult {
    pub schema: u16,
    pub daemon_path: PathBuf,
    pub executable_digest_sha256: String,
    pub bootstrap_tree_sha256: String,
    pub selected_release: String,
    pub data_dir: PathBuf,
    pub origin_host_epoch: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationBootstrapIdentity {
    pub schema: u16,
    pub bootstrap_tree_sha256: String,
    pub executable_digest_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ActivationOperationState {
    Accepted,
    Preparing,
    PublicationIntent,
    Completed,
    FailedBeforePublication,
    Uncertain,
    RecoveredPublication,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationOperationRecord {
    pub schema: u16,
    pub operation_id: String,
    pub command_sha256: String,
    pub command: ActivationCommand,
    pub platform: String,
    pub version: String,
    pub origin_host_epoch: u64,
    pub effect_issuer_process_id: u32,
    pub selected_release: String,
    pub state: ActivationOperationState,
    pub automatic_update_helper_verified: bool,
    pub result: Option<ActivationResult>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ActivationInspectionState {
    Busy,
    Unavailable,
    EpochMismatch,
    NotClosed,
    SelectionMismatch,
    SelectionVerified,
    PublishedRecoveryRequired,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationOperationInspection {
    pub state: ActivationInspectionState,
    pub observed_host_epoch: Option<u64>,
    pub observed_closed_ingress: Option<bool>,
    pub selected_release: Option<String>,
    pub automatic_update_helper_verified: Option<bool>,
    pub adoption_eligible: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationOperationStatus {
    pub schema: u16,
    pub operation: ActivationOperationRecord,
    pub inspection: ActivationOperationInspection,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationOperationPlan {
    pub schema: u16,
    pub operation_id: String,
    pub command_sha256: String,
    pub command: ActivationCommand,
    pub platform: String,
    pub version: String,
    pub selected_release: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(
    tag = "outcome",
    content = "body",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum ActivationMaintenanceReply {
    Capabilities {
        schema: u16,
        capabilities: Vec<String>,
    },
    BootstrapIdentity(ActivationBootstrapIdentity),
    Activated(ActivationResult),
    Operation(Box<ActivationOperationStatus>),
    Planned(Box<ActivationOperationPlan>),
    Origin(ActivationOriginStatus),
    Error {
        code: String,
        message: String,
    },
}
