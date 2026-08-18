//! Pure fencing for recoverable provider compaction.

use gent_types::AgentChatCompactionFact;

/// Pure state for one run's ordered normalized compaction facts.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AgentChatCompactionState {
    pub active_turn_id: Option<String>,
    pub recovered_turn_id: Option<String>,
    pub last_cursor: Option<u64>,
}

/// A provider-independent recovery decision. The runtime must persist a selected child run
/// before starting a fresh provider-native session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentChatCompactionEffect {
    None,
    RecoverFromFrozenLedger {
        event_id: String,
        source_cursor: u64,
        turn_id: String,
    },
    Rejected(AgentChatCompactionRejection),
}

/// A fact was ignored safely; none of these outcomes makes a conversation terminal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentChatCompactionRejection {
    EmptyTurnId,
    InvalidEventId,
    StaleCursor,
    CompactionAlreadyActive,
    NoMatchingCompaction,
    AlreadyRecovered,
    NonRecoverableFailure,
}

/// Applies one cursor-ordered normalized compaction fact without storage or provider access.
///
/// Only the explicitly normalized `TooFewGroups` failure requests recovery from Gent's frozen
/// durable history, never reuse of the failed provider-native session. Other provider failures
/// are retained as facts but cannot create a child run. Repeated facts cannot request another
/// child run.
#[must_use]
pub fn reduce_agent_chat_compaction(
    mut state: AgentChatCompactionState,
    cursor: u64,
    fact: &AgentChatCompactionFact,
) -> (AgentChatCompactionState, AgentChatCompactionEffect) {
    if state.last_cursor.is_some_and(|last| cursor <= last) {
        return (
            state,
            AgentChatCompactionEffect::Rejected(AgentChatCompactionRejection::StaleCursor),
        );
    }
    state.last_cursor = Some(cursor);
    let turn_id = turn_id(fact);
    let event_id = event_id(fact);
    if turn_id.trim().is_empty() {
        return (
            state,
            AgentChatCompactionEffect::Rejected(AgentChatCompactionRejection::EmptyTurnId),
        );
    }
    if event_id.trim().is_empty() || event_id.len() > 256 {
        return (
            state,
            AgentChatCompactionEffect::Rejected(AgentChatCompactionRejection::InvalidEventId),
        );
    }
    match fact {
        AgentChatCompactionFact::Started { .. } => start(state, turn_id),
        AgentChatCompactionFact::Completed { .. } => complete(state, turn_id),
        AgentChatCompactionFact::Failed { failure, .. } => {
            fail(state, event_id, cursor, turn_id, *failure)
        }
    }
}

fn turn_id(fact: &AgentChatCompactionFact) -> &str {
    match fact {
        AgentChatCompactionFact::Started { turn_id, .. }
        | AgentChatCompactionFact::Completed { turn_id, .. }
        | AgentChatCompactionFact::Failed { turn_id, .. } => turn_id,
    }
}

fn event_id(fact: &AgentChatCompactionFact) -> &str {
    match fact {
        AgentChatCompactionFact::Started { event_id, .. }
        | AgentChatCompactionFact::Completed { event_id, .. }
        | AgentChatCompactionFact::Failed { event_id, .. } => event_id,
    }
}

fn start(
    mut state: AgentChatCompactionState,
    turn_id: &str,
) -> (AgentChatCompactionState, AgentChatCompactionEffect) {
    match state.active_turn_id.as_deref() {
        None => {
            state.active_turn_id = Some(turn_id.into());
            state.recovered_turn_id = None;
            (state, AgentChatCompactionEffect::None)
        }
        Some(active) if active == turn_id => (state, AgentChatCompactionEffect::None),
        Some(_) => (
            state,
            AgentChatCompactionEffect::Rejected(
                AgentChatCompactionRejection::CompactionAlreadyActive,
            ),
        ),
    }
}

fn complete(
    mut state: AgentChatCompactionState,
    turn_id: &str,
) -> (AgentChatCompactionState, AgentChatCompactionEffect) {
    if state.active_turn_id.as_deref() != Some(turn_id) {
        return (
            state,
            AgentChatCompactionEffect::Rejected(AgentChatCompactionRejection::NoMatchingCompaction),
        );
    }
    state.active_turn_id = None;
    (state, AgentChatCompactionEffect::None)
}

fn fail(
    mut state: AgentChatCompactionState,
    event_id: &str,
    source_cursor: u64,
    turn_id: &str,
    failure: gent_types::AgentChatCompactionFailure,
) -> (AgentChatCompactionState, AgentChatCompactionEffect) {
    if state.recovered_turn_id.as_deref() == Some(turn_id) {
        return (
            state,
            AgentChatCompactionEffect::Rejected(AgentChatCompactionRejection::AlreadyRecovered),
        );
    }
    if state.active_turn_id.as_deref() != Some(turn_id) {
        return (
            state,
            AgentChatCompactionEffect::Rejected(AgentChatCompactionRejection::NoMatchingCompaction),
        );
    }
    state.active_turn_id = None;
    if failure != gent_types::AgentChatCompactionFailure::TooFewGroups {
        return (
            state,
            AgentChatCompactionEffect::Rejected(
                AgentChatCompactionRejection::NonRecoverableFailure,
            ),
        );
    }
    state.recovered_turn_id = Some(turn_id.into());
    (
        state,
        AgentChatCompactionEffect::RecoverFromFrozenLedger {
            event_id: event_id.into(),
            turn_id: turn_id.into(),
            source_cursor,
        },
    )
}
