//! Secret-free, daemon-only boundary for a private Claurst execution bridge.
//!
//! This port transports already-normalized facts. It deliberately does not model
//! connection configuration, credentials, provider requests, or raw provider payloads.

use async_trait::async_trait;
use gent_types::{
    GoalContractError, GoalProjection, GoalRecord, NormalizedLifecycleSignal,
    NormalizedProviderEvent,
};

use crate::PortError;

/// Hard upper bound for facts a private bridge may return from one drain call.
pub const MAX_PRIVATE_CLAURST_DRAIN_FACTS: u16 = 64;

/// Daemon-owned durable identity for one private bridge event source.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ClaurstSourceId(pub String);

/// An opaque native session bound to exactly one daemon-owned run and source.
///
/// This value remains on the daemon/private-bridge boundary and is never a client projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaurstSessionBinding {
    pub run_id: String,
    pub source_id: ClaurstSourceId,
    pub opaque_session_id: String,
}

/// Secret-free active-goal context for a private Claurst bridge.
///
/// It intentionally carries no endpoint, credential, routing, provider-native session, plan, or
/// provider result. A future daemon composition may deliver this DTO only after the goal ledger
/// has validated the active record and reserved the run named here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaurstGoalProjection {
    pub run_id: String,
    pub source_id: ClaurstSourceId,
    pub goal: GoalProjection,
}

impl ClaurstGoalProjection {
    /// Maps exactly one durable active goal to its daemon-owned private source.
    ///
    /// # Errors
    /// Returns an error if the record is not an active valid goal or the source is empty.
    pub fn from_active_goal(
        source_id: ClaurstSourceId,
        goal: &GoalRecord,
    ) -> Result<Self, GoalContractError> {
        if source_id.0.is_empty() {
            return Err(GoalContractError::InvalidMetadata);
        }
        let goal = GoalProjection::from_active(goal)?;
        Ok(Self {
            run_id: goal.binding().run_id.0.clone(),
            source_id,
            goal,
        })
    }
}

/// Immutable recovery position, without exposing the private checkpoint material itself.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaurstCheckpoint {
    pub run_id: String,
    pub source_id: ClaurstSourceId,
    pub cursor: u64,
    pub state_digest_sha256: String,
}

/// One ordered, provider-neutral fact from a daemon-owned private source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaurstNormalizedFact {
    pub source_id: ClaurstSourceId,
    pub cursor: u64,
    pub value: ClaurstFactValue,
}

/// Public-wire-fact-equivalent content accepted from a private bridge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClaurstFactValue {
    Event(NormalizedProviderEvent),
    Lifecycle(NormalizedLifecycleSignal),
}

/// Controlled, content-free terminal settlement for a private source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaurstTerminal {
    Completed,
    Interrupted,
    Failed {
        classification: ClaurstFailureClassification,
    },
}

/// A fixed failure vocabulary that cannot carry provider error text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaurstFailureClassification {
    Authentication,
    Authorization,
    Protocol,
    Unavailable,
    Internal,
}

/// Bounded request to drain facts strictly after one durable source cursor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaurstDrainRequest {
    pub run_id: String,
    pub source_id: ClaurstSourceId,
    pub after_cursor: u64,
    pub limit: u16,
}

impl ClaurstDrainRequest {
    /// Returns whether this request stays within the fixed bridge backpressure bound.
    #[must_use]
    pub const fn is_bounded(&self) -> bool {
        self.limit > 0 && self.limit <= MAX_PRIVATE_CLAURST_DRAIN_FACTS
    }
}

/// A drain result with an optional newly observed session and terminal settlement.
///
/// `session_binding` is private daemon state, never suitable for a public transport response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaurstDrainBatch {
    pub facts: Vec<ClaurstNormalizedFact>,
    pub checkpoint: Option<ClaurstCheckpoint>,
    pub session_binding: Option<ClaurstSessionBinding>,
    pub terminal: Option<ClaurstTerminal>,
}

/// Private sidecar ingress for normalized Claurst execution facts.
///
/// A daemon binds its durable source before draining it. The bridge must not return more than the
/// request limit, return facts at or before `after_cursor`, or emit facts after settlement.
#[async_trait]
pub trait PrivateClaurstBridge: Send + Sync {
    /// Associates an opaque private session with an already-reserved daemon run.
    async fn bind_session(&self, binding: ClaurstSessionBinding) -> Result<(), PortError>;

    /// Drains one bounded ordered batch from one daemon-owned private source.
    async fn drain(&self, request: ClaurstDrainRequest) -> Result<ClaurstDrainBatch, PortError>;
}
