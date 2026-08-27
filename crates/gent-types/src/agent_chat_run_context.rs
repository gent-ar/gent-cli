//! Immutable durable context boundary for one provider-neutral agent-chat run.

use serde::{Deserialize, Serialize};

use crate::{AgentChatConversationId, AgentChatRunId, ContextPolicy};

/// The durable operation that created a run's context boundary.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum AgentChatRunContextOrigin {
    /// The conversation's original run; it inherited no prior provider-neutral history.
    Root,
    /// An immutable provider/model/mode selection child.
    SelectionSwitch,
    /// An immutable implementation child approved from a reviewed plan.
    ReviewedPlan,
    /// The root run of a new conversation seeded from another conversation's prior messages.
    Forked,
    /// An immutable child run whose context resumes at an earlier durable checkpoint.
    CheckpointRestore,
}

/// Context policy and frozen history ordinal fixed when one run was durably created.
///
/// This value intentionally contains neither provider-native sessions nor history text. Child
/// origins always require a fresh provider-native session, even where their context is empty.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentChatRunContext {
    pub conversation_id: AgentChatConversationId,
    pub run_id: AgentChatRunId,
    pub origin: AgentChatRunContextOrigin,
    pub context_policy: ContextPolicy,
    pub context_through_ordinal: u64,
}

impl AgentChatRunContext {
    /// Returns whether this durable child must start without any provider-native resume state.
    #[must_use]
    pub const fn requires_fresh_provider_session(&self) -> bool {
        !matches!(self.origin, AgentChatRunContextOrigin::Root)
    }
}
