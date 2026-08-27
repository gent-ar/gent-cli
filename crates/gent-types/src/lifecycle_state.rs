//! Provider-neutral lifecycle values used by durable projection and status transport.

use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeMap};

/// Durable root-turn state. Detached work never changes this state to completed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TurnPhase {
    Processing,
    WaitingPermission,
    WaitingQuestion,
    Compacting,
    Ready,
    Interrupted,
    Dead,
    Failed,
}

/// Explicit root activity fact. It is independent from durable turn phase.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RootActivity {
    Generating,
    Waiting,
    #[default]
    Idle,
}

impl RootActivity {
    #[must_use]
    pub const fn is_generating(self) -> bool {
        matches!(self, Self::Generating)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkPhase {
    Pending,
    Running,
    WaitingPermission,
    Done,
    Failed,
    Interrupted,
}

impl WorkPhase {
    #[must_use]
    pub const fn is_live(&self) -> bool {
        matches!(
            self,
            Self::Pending | Self::Running | Self::WaitingPermission
        )
    }
}

/// A complete volatile status sent over transport, never transcript content.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConversationProcessingStatus {
    Processing,
    #[default]
    Idle,
}

impl ConversationProcessingStatus {
    #[must_use]
    pub const fn is_processing(self) -> bool {
        matches!(self, Self::Processing)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConversationWorkStatus {
    #[default]
    None,
    Active,
    Waiting,
}

impl ConversationWorkStatus {
    #[must_use]
    pub const fn is_live(self) -> bool {
        !matches!(self, Self::None)
    }

    #[must_use]
    pub const fn is_waiting(self) -> bool {
        matches!(self, Self::Waiting)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConversationAttentionStatus {
    Required,
    #[default]
    Clear,
}

impl ConversationAttentionStatus {
    #[must_use]
    pub const fn is_required(self) -> bool {
        matches!(self, Self::Required)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConversationErrorStatus {
    Error,
    #[default]
    Clear,
}

impl ConversationErrorStatus {
    #[must_use]
    pub const fn has_error(self) -> bool {
        matches!(self, Self::Error)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConversationLiveStatus {
    pub cursor: u64,
    pub processing: ConversationProcessingStatus,
    pub subagent_work: ConversationWorkStatus,
    pub command_work: ConversationWorkStatus,
    pub attention: ConversationAttentionStatus,
    pub error: ConversationErrorStatus,
}

impl ConversationLiveStatus {
    #[must_use]
    pub const fn is_processing(&self) -> bool {
        self.processing.is_processing()
    }

    #[must_use]
    pub const fn is_waiting_for_subagents(&self) -> bool {
        self.subagent_work.is_waiting()
    }

    #[must_use]
    pub const fn has_live_subagent_work(&self) -> bool {
        self.subagent_work.is_live()
    }

    #[must_use]
    pub const fn is_waiting_for_command(&self) -> bool {
        self.command_work.is_waiting()
    }

    #[must_use]
    pub const fn has_live_command_work(&self) -> bool {
        self.command_work.is_live()
    }

    #[must_use]
    pub const fn needs_attention(&self) -> bool {
        self.attention.is_required()
    }

    #[must_use]
    pub const fn has_error(&self) -> bool {
        self.error.has_error()
    }
}

impl Serialize for ConversationLiveStatus {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(8))?;
        map.serialize_entry("cursor", &self.cursor)?;
        map.serialize_entry("isProcessing", &self.processing.is_processing())?;
        map.serialize_entry("isWaitingForSubagents", &self.subagent_work.is_waiting())?;
        map.serialize_entry("hasLiveSubagentWork", &self.subagent_work.is_live())?;
        map.serialize_entry("isWaitingForCommand", &self.command_work.is_waiting())?;
        map.serialize_entry("hasLiveCommandWork", &self.command_work.is_live())?;
        map.serialize_entry("needsAttention", &self.attention.is_required())?;
        map.serialize_entry("hasError", &self.error.has_error())?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for ConversationLiveStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = serde_json::Value::deserialize(deserializer)?;
        Ok(Self {
            cursor: wire_u64::<D::Error>(&wire, "cursor")?,
            processing: if wire_bool::<D::Error>(&wire, "isProcessing")? {
                ConversationProcessingStatus::Processing
            } else {
                ConversationProcessingStatus::Idle
            },
            subagent_work: work_status(
                wire_bool::<D::Error>(&wire, "hasLiveSubagentWork")?,
                wire_bool::<D::Error>(&wire, "isWaitingForSubagents")?,
            ),
            command_work: work_status(
                wire_bool::<D::Error>(&wire, "hasLiveCommandWork")?,
                wire_bool::<D::Error>(&wire, "isWaitingForCommand")?,
            ),
            attention: if wire_bool::<D::Error>(&wire, "needsAttention")? {
                ConversationAttentionStatus::Required
            } else {
                ConversationAttentionStatus::Clear
            },
            error: if wire_bool::<D::Error>(&wire, "hasError")? {
                ConversationErrorStatus::Error
            } else {
                ConversationErrorStatus::Clear
            },
        })
    }
}

fn wire_u64<E: serde::de::Error>(value: &serde_json::Value, key: &str) -> Result<u64, E> {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| E::custom(format!("invalid conversation live status {key}")))
}

fn wire_bool<E: serde::de::Error>(value: &serde_json::Value, key: &str) -> Result<bool, E> {
    value
        .get(key)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| E::custom(format!("invalid conversation live status {key}")))
}

const fn work_status(live: bool, waiting: bool) -> ConversationWorkStatus {
    if live && waiting {
        ConversationWorkStatus::Waiting
    } else if live {
        ConversationWorkStatus::Active
    } else {
        ConversationWorkStatus::None
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ConversationAttentionStatus, ConversationErrorStatus, ConversationLiveStatus,
        ConversationProcessingStatus, ConversationWorkStatus,
    };

    #[test]
    fn status_keeps_its_boolean_wire_shape() {
        let status = ConversationLiveStatus {
            cursor: 8,
            processing: ConversationProcessingStatus::Processing,
            subagent_work: ConversationWorkStatus::Waiting,
            command_work: ConversationWorkStatus::Active,
            attention: ConversationAttentionStatus::Required,
            error: ConversationErrorStatus::Error,
        };
        assert_eq!(
            serde_json::to_value(&status).unwrap(),
            serde_json::json!({
                "cursor": 8,
                "isProcessing": true,
                "isWaitingForSubagents": true,
                "hasLiveSubagentWork": true,
                "isWaitingForCommand": false,
                "hasLiveCommandWork": true,
                "needsAttention": true,
                "hasError": true
            })
        );
        assert_eq!(
            serde_json::from_value::<ConversationLiveStatus>(serde_json::to_value(status).unwrap())
                .unwrap()
                .command_work,
            ConversationWorkStatus::Active
        );
    }
}
