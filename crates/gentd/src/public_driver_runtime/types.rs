//! Shared public-driver runtime values, kept separate from composition to bound source size.

use gent_drivers::{SessionEffect, public_protocol::PublicWireFact};
use gent_runtime::{
    AgentChatTranscriptAppendRequest, AgentChatTranscriptAppendResult, ConversationActivityResult,
    ProviderActivityFact,
};
use gent_types::RunLiveStatus;

/// A fact emitted at the public-driver process boundary with its own durable source identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PublicDriverFact {
    SessionEffect {
        event_id: String,
        effect: SessionEffect,
    },
    PublicWire {
        event_id: String,
        fact: PublicWireFact,
    },
    Activity(ProviderActivityFact),
    /// A daemon-mapped transcript fact. The daemon, not the driver, supplies durable IDs.
    Transcript(AgentChatTranscriptAppendRequest),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PublicDriverFactResult {
    Lifecycle(Option<RunLiveStatus>),
    Activity(ConversationActivityResult),
    Transcript(AgentChatTranscriptAppendResult),
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum PublicDriversRuntimeError {
    #[error("the observer profile cannot construct public-driver authority")]
    ObserverProfile,
    #[error("the approved compatibility manifest is unavailable")]
    CompatibilityManifestUnavailable,
    #[error("the approved compatibility manifest digest does not match the verified cache")]
    CompatibilityManifestMismatch,
}
