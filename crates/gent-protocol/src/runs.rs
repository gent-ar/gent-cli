//! Typed public-provider run requests. Private providers are deliberately unrepresentable.

use gent_types::HostEpoch;
use serde::{Deserialize, Serialize};

use crate::DependencyProvider;

/// Starts a new root run after the daemon has entered an explicit authority mode.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicRunStartRequest {
    pub run_id: String,
    pub coordinator_id: String,
    pub host_epoch: HostEpoch,
    pub provider: DependencyProvider,
    pub executable: String,
    pub version: String,
    pub compatibility_entry: String,
}

/// Resumes an existing run using its previously persisted immutable executable lock.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicRunResumeRequest {
    pub run_id: String,
    pub coordinator_id: String,
    pub host_epoch: HostEpoch,
    pub session_id: String,
}

/// Requests whole-process-tree interruption for a run owned by this coordinator.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicRunInterruptRequest {
    pub run_id: String,
    pub coordinator_id: String,
    pub host_epoch: HostEpoch,
}

/// Observable outcome of a provider-run lifecycle operation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PublicRunOutcome {
    Started,
    Resumed,
    Interrupted,
    Denied,
    ProviderChanged,
    LeaseContended,
}

/// A typed lifecycle response; ordinary storage and validation failures remain protocol errors.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicRunResponse {
    pub run_id: String,
    pub outcome: PublicRunOutcome,
}
