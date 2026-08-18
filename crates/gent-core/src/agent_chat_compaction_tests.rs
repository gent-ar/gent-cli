use gent_types::{AgentChatCompactionFact, AgentChatCompactionFailure};

use super::{
    AgentChatCompactionEffect, AgentChatCompactionRejection, AgentChatCompactionState,
    reduce_agent_chat_compaction,
};

fn started() -> AgentChatCompactionFact {
    AgentChatCompactionFact::Started {
        turn_id: "turn-1".into(),
    }
}

#[test]
fn too_few_groups_recovers_from_frozen_ledger_once() {
    let (state, effect) =
        reduce_agent_chat_compaction(AgentChatCompactionState::default(), 1, &started());
    assert_eq!(effect, AgentChatCompactionEffect::None);
    let failed = AgentChatCompactionFact::Failed {
        turn_id: "turn-1".into(),
        failure: AgentChatCompactionFailure::TooFewGroups,
    };
    let (state, effect) = reduce_agent_chat_compaction(state, 2, &failed);
    assert_eq!(
        effect,
        AgentChatCompactionEffect::RecoverFromFrozenLedger {
            turn_id: "turn-1".into()
        }
    );
    assert_eq!(state.recovered_turn_id.as_deref(), Some("turn-1"));
    let (_, duplicate) = reduce_agent_chat_compaction(state, 3, &failed);
    assert_eq!(
        duplicate,
        AgentChatCompactionEffect::Rejected(AgentChatCompactionRejection::AlreadyRecovered)
    );
}

#[test]
fn every_normalized_failure_recovers_without_a_provider_error() {
    let (state, _) =
        reduce_agent_chat_compaction(AgentChatCompactionState::default(), 1, &started());
    let (state, effect) = reduce_agent_chat_compaction(
        state,
        2,
        &AgentChatCompactionFact::Failed {
            turn_id: "turn-1".into(),
            failure: AgentChatCompactionFailure::ProviderFailed,
        },
    );
    assert!(matches!(
        effect,
        AgentChatCompactionEffect::RecoverFromFrozenLedger { .. }
    ));
    assert_eq!(state.active_turn_id, None);
}

#[test]
fn stale_or_wrong_turn_facts_cannot_trigger_a_recovery() {
    let (state, _) =
        reduce_agent_chat_compaction(AgentChatCompactionState::default(), 3, &started());
    let (_, stale_effect) = reduce_agent_chat_compaction(
        state.clone(),
        3,
        &AgentChatCompactionFact::Failed {
            turn_id: "turn-1".into(),
            failure: AgentChatCompactionFailure::TooFewGroups,
        },
    );
    assert_eq!(
        stale_effect,
        AgentChatCompactionEffect::Rejected(AgentChatCompactionRejection::StaleCursor)
    );
    let (_, wrong) = reduce_agent_chat_compaction(
        state,
        4,
        &AgentChatCompactionFact::Failed {
            turn_id: "turn-2".into(),
            failure: AgentChatCompactionFailure::TooFewGroups,
        },
    );
    assert_eq!(
        wrong,
        AgentChatCompactionEffect::Rejected(AgentChatCompactionRejection::NoMatchingCompaction)
    );
}

#[test]
fn malformed_fact_consumes_its_cursor_without_recovering() {
    let (state, invalid) = reduce_agent_chat_compaction(
        AgentChatCompactionState::default(),
        5,
        &AgentChatCompactionFact::Started {
            turn_id: " ".into(),
        },
    );
    assert_eq!(
        invalid,
        AgentChatCompactionEffect::Rejected(AgentChatCompactionRejection::EmptyTurnId)
    );
    let (_, duplicate) = reduce_agent_chat_compaction(state, 5, &started());
    assert_eq!(
        duplicate,
        AgentChatCompactionEffect::Rejected(AgentChatCompactionRejection::StaleCursor)
    );
}

#[test]
fn completion_only_clears_the_matching_active_compaction() {
    let (state, _) =
        reduce_agent_chat_compaction(AgentChatCompactionState::default(), 1, &started());
    let (state, effect) = reduce_agent_chat_compaction(
        state,
        2,
        &AgentChatCompactionFact::Completed {
            turn_id: "turn-1".into(),
        },
    );
    assert_eq!(effect, AgentChatCompactionEffect::None);
    assert_eq!(state.active_turn_id, None);
    let (_, wrong) = reduce_agent_chat_compaction(
        state,
        3,
        &AgentChatCompactionFact::Completed {
            turn_id: "turn-2".into(),
        },
    );
    assert_eq!(
        wrong,
        AgentChatCompactionEffect::Rejected(AgentChatCompactionRejection::NoMatchingCompaction)
    );
}

#[test]
fn duplicate_start_is_inert_but_another_turn_is_rejected() {
    let (state, _) =
        reduce_agent_chat_compaction(AgentChatCompactionState::default(), 1, &started());
    let (state, duplicate) = reduce_agent_chat_compaction(state, 2, &started());
    assert_eq!(duplicate, AgentChatCompactionEffect::None);
    let (_, competing) = reduce_agent_chat_compaction(
        state,
        3,
        &AgentChatCompactionFact::Started {
            turn_id: "turn-2".into(),
        },
    );
    assert_eq!(
        competing,
        AgentChatCompactionEffect::Rejected(AgentChatCompactionRejection::CompactionAlreadyActive)
    );
}

#[test]
fn a_later_turn_can_recover_after_the_previous_turn_did() {
    let (state, _) =
        reduce_agent_chat_compaction(AgentChatCompactionState::default(), 1, &started());
    let (state, _) = reduce_agent_chat_compaction(
        state,
        2,
        &AgentChatCompactionFact::Failed {
            turn_id: "turn-1".into(),
            failure: AgentChatCompactionFailure::TooFewGroups,
        },
    );
    let (state, _) = reduce_agent_chat_compaction(
        state,
        3,
        &AgentChatCompactionFact::Started {
            turn_id: "turn-2".into(),
        },
    );
    let (_, effect) = reduce_agent_chat_compaction(
        state,
        4,
        &AgentChatCompactionFact::Failed {
            turn_id: "turn-2".into(),
            failure: AgentChatCompactionFailure::ProviderFailed,
        },
    );
    assert_eq!(
        effect,
        AgentChatCompactionEffect::RecoverFromFrozenLedger {
            turn_id: "turn-2".into()
        }
    );
}
