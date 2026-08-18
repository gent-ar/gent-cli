//! Secret-free, daemon-only boundary for a private Claurst execution bridge.
//!
//! This port transports already-normalized facts. It deliberately does not model
//! connection configuration, credentials, provider requests, or raw provider payloads.

use async_trait::async_trait;
use gent_types::{
    FrozenConversationContext, GoalContractError, GoalProjection, GoalRecord,
    NormalizedLifecycleSignal, NormalizedProviderEvent,
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

/// Daemon-owned initial input for one private Claurst session.
///
/// This is deliberately an in-process private-port value, not a public IPC frame. It contains
/// only Gent-owned normalized prompt/context/goal inputs; endpoint, credentials, routing, raw
/// provider plans, and native options remain outside the public repository contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaurstStartRequest {
    pub run_id: String,
    pub source_id: ClaurstSourceId,
    pub turn_id: String,
    pub prompt: String,
    pub context: FrozenConversationContext,
    pub goal: Option<ClaurstGoalProjection>,
}

impl ClaurstStartRequest {
    /// Validates that one fresh private session stays bound to its durable Gent identities.
    ///
    /// # Errors
    /// Returns an error before a bridge receives malformed identities, a cross-run context, or a
    /// mismatched active goal. This does not validate provider-native configuration.
    pub fn validate(&self) -> Result<(), GoalContractError> {
        if self.run_id.trim().is_empty()
            || self.source_id.0.trim().is_empty()
            || self.turn_id.trim().is_empty()
            || self.prompt.trim().is_empty()
            || self.context.conversation_id.0.trim().is_empty()
        {
            return Err(GoalContractError::InvalidMetadata);
        }
        if let Some(goal) = &self.goal
            && (goal.run_id != self.run_id
                || goal.source_id != self.source_id
                || goal.goal.binding().conversation_id != self.context.conversation_id)
        {
            return Err(GoalContractError::InvalidMetadata);
        }
        Ok(())
    }
}

/// Daemon-owned follow-up input for an already bound private Claurst session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaurstSubmitRequest {
    pub binding: ClaurstSessionBinding,
    pub turn_id: String,
    pub prompt: String,
    pub goal: Option<ClaurstGoalProjection>,
}

impl ClaurstSubmitRequest {
    /// Validates one follow-up against its daemon-owned session binding.
    ///
    /// # Errors
    /// Returns before bridge submission for malformed identities or a goal from another source.
    pub fn validate(&self) -> Result<(), GoalContractError> {
        if self.binding.run_id.trim().is_empty()
            || self.binding.source_id.0.trim().is_empty()
            || self.binding.opaque_session_id.is_empty()
            || self.turn_id.trim().is_empty()
            || self.prompt.trim().is_empty()
        {
            return Err(GoalContractError::InvalidMetadata);
        }
        if let Some(goal) = &self.goal
            && (goal.run_id != self.binding.run_id || goal.source_id != self.binding.source_id)
        {
            return Err(GoalContractError::InvalidMetadata);
        }
        Ok(())
    }
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
    /// Starts one fresh private session from daemon-owned normalized input.
    ///
    /// A private sidecar implementation may use its own private configuration, but it must
    /// return only an opaque session binding for the requested run/source. The default keeps the
    /// bridge unavailable until a separately reviewed private implementation is composed.
    async fn start(
        &self,
        request: ClaurstStartRequest,
    ) -> Result<ClaurstSessionBinding, PortError> {
        let _ = request;
        Err(PortError::Unavailable("private Claurst start".into()))
    }

    /// Associates an opaque private session with an already-reserved daemon run.
    async fn bind_session(&self, binding: ClaurstSessionBinding) -> Result<(), PortError>;

    /// Submits a daemon-owned follow-up to one already bound private session.
    ///
    /// The default keeps follow-up unavailable until the private bridge implements the same
    /// lifecycle contract as start/drain; no public client can invoke this port directly.
    async fn submit(&self, request: ClaurstSubmitRequest) -> Result<(), PortError> {
        let _ = request;
        Err(PortError::Unavailable("private Claurst submit".into()))
    }

    /// Drains one bounded ordered batch from one daemon-owned private source.
    async fn drain(&self, request: ClaurstDrainRequest) -> Result<ClaurstDrainBatch, PortError>;
}

#[cfg(test)]
#[path = "private_claurst_bridge_tests.rs"]
mod tests;
