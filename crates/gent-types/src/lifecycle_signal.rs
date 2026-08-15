//! Additive lifecycle facts emitted alongside provider-neutral content events.

use serde::{Deserialize, Serialize};

use crate::{TurnPhase, WorkPhase};

/// A provider-neutral lifecycle fact with no transcript or provider-session content.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum NormalizedLifecycleSignal {
    RootPhase {
        phase: TurnPhase,
    },
    ChildPhase {
        child_id: String,
        phase: WorkPhase,
    },
    CommandPhase {
        command_id: String,
        phase: WorkPhase,
    },
    AttentionRequired,
    AttentionCleared,
}
