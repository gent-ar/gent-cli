use gent_core::{
    AgentChatCompactionEffect, AgentChatCompactionState, reduce_agent_chat_compaction,
};
use gent_ports::{ConversationContentReader, ConversationLedger, Ledger};
use gent_runtime::{
    AgentChatCompactionRecoveryAuthority, AgentChatCompactionRecoveryRequest,
    AgentChatCompactionRecoveryResult, AgentChatCompactionRecoveryService,
    AgentChatConversationAuthority, AgentChatConversationRequest, AgentChatConversationResult,
    AgentChatConversationService, AgentChatPromptAuthority, AgentChatPromptRequest,
    AgentChatPromptResult, AgentChatPromptService,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatCompactionFact, AgentChatCompactionFailure, AgentChatConversationId, AgentChatEffort,
    AgentChatMode, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, HostEpoch, ReceiptId,
};

fn selection() -> AgentChatSelection {
    AgentChatSelection {
        provider: AgentChatProvider::Codex,
        model: "gpt-5.6".into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Agent,
    }
}

fn conversation(ledger: SqliteLedger) -> (AgentChatConversationId, AgentChatRunId) {
    let created =
        AgentChatConversationService::new(ledger, AgentChatConversationAuthority::Approved)
            .create(&AgentChatConversationRequest {
                request_id: AgentChatRequestId("conversation".into()),
                receipt_id: ReceiptId("conversation-receipt".into()),
                host_epoch: HostEpoch(1),
                selection: selection(),
            })
            .unwrap();
    let AgentChatConversationResult::Created(created) = created else {
        panic!("approved creation")
    };
    (created.conversation_id, created.run_id)
}

fn save(ledger: SqliteLedger, conversation_id: AgentChatConversationId) {
    let saved = AgentChatPromptService::new(ledger, AgentChatPromptAuthority::Approved)
        .submit(&AgentChatPromptRequest {
            request_id: AgentChatRequestId("prompt".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id,
            disposition: AgentChatPromptDisposition::Send,
            text: "retain this context".into(),
        })
        .unwrap();
    assert!(matches!(saved, AgentChatPromptResult::Saved(_)));
}

fn recovery_effect() -> AgentChatCompactionEffect {
    let (state, _) = reduce_agent_chat_compaction(
        AgentChatCompactionState::default(),
        1,
        &AgentChatCompactionFact::Started {
            event_id: "provider-event-1".into(),
            turn_id: "turn-1".into(),
        },
    );
    let (_, effect) = reduce_agent_chat_compaction(
        state,
        2,
        &AgentChatCompactionFact::Failed {
            event_id: "provider-event-2".into(),
            turn_id: "turn-1".into(),
            failure: AgentChatCompactionFailure::TooFewGroups,
        },
    );
    effect
}

fn terminal_failure_effect() -> AgentChatCompactionEffect {
    let (_, effect) = reduce_agent_chat_compaction(
        AgentChatCompactionState::default(),
        2,
        &AgentChatCompactionFact::Failed {
            event_id: "provider-event-2".into(),
            turn_id: "turn-1".into(),
            failure: AgentChatCompactionFailure::TooFewGroups,
        },
    );
    effect
}

#[test]
fn failed_compaction_creates_one_fresh_session_child_with_frozen_history() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let (conversation_id, parent_run_id) = conversation(ledger.clone());
    save(ledger.clone(), conversation_id.clone());
    let service = AgentChatCompactionRecoveryService::new(
        ledger.clone(),
        AgentChatCompactionRecoveryAuthority::Approved,
    );
    let request = AgentChatCompactionRecoveryRequest {
        source_event_id: "provider-event-2".into(),
        source_cursor: 2,
        host_epoch: HostEpoch(1),
        conversation_id: conversation_id.clone(),
        parent_run_id: parent_run_id.clone(),
        selection: selection(),
    };
    let first = service.apply(&request, &recovery_effect()).unwrap();
    let second = service.apply(&request, &recovery_effect()).unwrap();
    assert_eq!(first, second);
    let AgentChatCompactionRecoveryResult::Recovered(switched) = first else {
        panic!("recovery child")
    };
    assert_eq!(switched.parent_run_id, parent_run_id);
    assert_eq!(switched.context_through_ordinal, 1);
    assert!(
        ledger
            .find_run_session_binding(&switched.run_id.0)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        ledger
            .list_conversation_runs(&conversation_id.0)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        ledger
            .read_conversation_content(&conversation_id.0, Some(2), 2)
            .unwrap()
            .entries
            .len(),
        1
    );
}

#[test]
fn terminal_compaction_failure_recovers_when_the_provider_omits_started() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let (conversation_id, parent_run_id) = conversation(ledger.clone());
    save(ledger.clone(), conversation_id.clone());
    let service = AgentChatCompactionRecoveryService::new(
        ledger.clone(),
        AgentChatCompactionRecoveryAuthority::Approved,
    );
    let recovered = service
        .apply(
            &AgentChatCompactionRecoveryRequest {
                source_event_id: "provider-event-2".into(),
                source_cursor: 2,
                host_epoch: HostEpoch(1),
                conversation_id: conversation_id.clone(),
                parent_run_id,
                selection: selection(),
            },
            &terminal_failure_effect(),
        )
        .unwrap();
    let AgentChatCompactionRecoveryResult::Recovered(child) = recovered else {
        panic!("terminal failure must create a recovery child")
    };
    assert_eq!(child.context_through_ordinal, 1);
    assert!(
        ledger
            .find_run_session_binding(&child.run_id.0)
            .unwrap()
            .is_none()
    );
}

#[test]
fn observer_and_non_recovery_effects_cannot_write() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let (conversation_id, parent_run_id) = conversation(ledger.clone());
    let request = AgentChatCompactionRecoveryRequest {
        source_event_id: "provider-event-2".into(),
        source_cursor: 2,
        host_epoch: HostEpoch(1),
        conversation_id: conversation_id.clone(),
        parent_run_id,
        selection: selection(),
    };
    let observer = AgentChatCompactionRecoveryService::new(
        ledger.clone(),
        AgentChatCompactionRecoveryAuthority::Observer,
    );
    assert_eq!(
        observer.apply(&request, &recovery_effect()).unwrap(),
        AgentChatCompactionRecoveryResult::DeniedObserver
    );
    let approved = AgentChatCompactionRecoveryService::new(
        ledger.clone(),
        AgentChatCompactionRecoveryAuthority::Approved,
    );
    assert_eq!(
        approved
            .apply(&request, &AgentChatCompactionEffect::None)
            .unwrap(),
        AgentChatCompactionRecoveryResult::Ignored
    );
    assert_eq!(
        ledger
            .list_conversation_runs(&conversation_id.0)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn recovery_refuses_an_uncorrelated_or_stale_source_before_reservation() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let (conversation_id, parent_run_id) = conversation(ledger.clone());
    let service = AgentChatCompactionRecoveryService::new(
        ledger.clone(),
        AgentChatCompactionRecoveryAuthority::Approved,
    );
    let durable_conversation_id = conversation_id.clone();
    let request = AgentChatCompactionRecoveryRequest {
        source_event_id: "another-event".into(),
        source_cursor: 2,
        host_epoch: HostEpoch(1),
        conversation_id,
        parent_run_id,
        selection: selection(),
    };
    assert!(service.apply(&request, &recovery_effect()).is_err());
    assert_eq!(
        ledger
            .list_conversation_runs(&durable_conversation_id.0)
            .unwrap()
            .len(),
        1
    );
}
