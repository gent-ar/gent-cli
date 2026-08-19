use gent_drivers::{
    PublicProvider,
    public_protocol::{PublicCompactionObservation, PublicWireFact, normalize_public_frame},
};
use gent_ports::{AgentChatLedger, ConversationLedger, Ledger, RunLease, RunSessionBinding};
use gent_runtime::AgentChatCompactionRecoveryAuthority;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatCompactionFact, AgentChatCompactionFailure, AgentChatConversationCreate,
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRunId,
    AgentChatSelection, HostEpoch, ReceiptId,
};

use crate::private_compaction_ingress::{
    PrivateCompactionIngress, PrivateCompactionObservationRequest, PrivateCompactionRequest,
    PrivateCompactionResult,
};

fn selection() -> AgentChatSelection {
    AgentChatSelection {
        provider: AgentChatProvider::Codex,
        model: "gpt-5.6".into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Agent,
    }
}

fn prepared_ledger() -> SqliteLedger {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_agent_chat_conversation(&AgentChatConversationCreate {
            receipt_id: ReceiptId("conversation-receipt".into()),
            idempotency_key: "conversation-key".into(),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            run_id: AgentChatRunId("run-a".into()),
            selection: selection(),
        })
        .unwrap();
    ledger
        .claim_run_lease(&RunLease {
            run_id: "run-a".into(),
            coordinator_id: "daemon-a".into(),
            host_epoch: HostEpoch(1),
        })
        .unwrap();
    ledger
        .save_run_session_binding(&RunSessionBinding {
            run_id: "run-a".into(),
            provider_session_id: "daemon-owned-session".into(),
        })
        .unwrap();
    ledger
}

fn failed() -> PrivateCompactionRequest {
    PrivateCompactionRequest {
        run_id: AgentChatRunId("run-a".into()),
        conversation_id: AgentChatConversationId("conversation-a".into()),
        coordinator_id: "daemon-a".into(),
        host_epoch: HostEpoch(1),
        selection: selection(),
        fact: AgentChatCompactionFact::Failed {
            event_id: "compaction-failed-1".into(),
            turn_id: "turn-a".into(),
            failure: AgentChatCompactionFailure::TooFewGroups,
        },
    }
}

fn observed(fact: PublicCompactionObservation) -> PrivateCompactionObservationRequest {
    PrivateCompactionObservationRequest {
        run_id: AgentChatRunId("run-a".into()),
        conversation_id: AgentChatConversationId("conversation-a".into()),
        coordinator_id: "daemon-a".into(),
        host_epoch: HostEpoch(1),
        selection: selection(),
        event_id: "compaction-observed-1".into(),
        turn_id: "turn-a".into(),
        observation: fact,
    }
}

#[test]
fn approved_ingress_persists_the_fact_before_creating_a_fresh_recovery_child() {
    let ledger = prepared_ledger();
    let mut ingress = PrivateCompactionIngress::new(
        ledger.clone(),
        AgentChatCompactionRecoveryAuthority::Approved,
    );
    let result = ingress.record(failed()).unwrap();
    let PrivateCompactionResult::Recovered(child) = result else {
        panic!("too-few-groups must create a fresh child");
    };
    assert_eq!(child.parent_run_id.0, "run-a");
    assert_eq!(child.context_through_ordinal, 0);
    let source = ledger.find_event("compaction-failed-1").unwrap().unwrap();
    assert_eq!(source.kind, "agentChatCompaction");
    assert!(!source.payload.to_string().contains("daemon-owned-session"));
    assert_eq!(
        ledger
            .list_conversation_runs("conversation-a")
            .unwrap()
            .len(),
        2
    );
    assert!(
        ledger
            .find_run_session_binding(&child.run_id.0)
            .unwrap()
            .is_none()
    );
}

#[test]
fn replay_after_ingress_restart_uses_the_durable_fact_and_same_child_receipt() {
    let ledger = prepared_ledger();
    let first = PrivateCompactionIngress::new(
        ledger.clone(),
        AgentChatCompactionRecoveryAuthority::Approved,
    )
    .record(failed())
    .unwrap();
    let PrivateCompactionResult::Recovered(first_child) = first else {
        panic!("initial compaction must create a child");
    };

    // A fresh ingress deliberately has no remembered reducer state. The durable source event
    // and deterministic recovery receipt must be sufficient to reproduce the same result.
    let replay = PrivateCompactionIngress::new(
        ledger.clone(),
        AgentChatCompactionRecoveryAuthority::Approved,
    )
    .record(failed())
    .unwrap();
    assert_eq!(replay, PrivateCompactionResult::Recovered(first_child));
    assert_eq!(
        ledger
            .list_conversation_runs("conversation-a")
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn observer_ingress_cannot_persist_or_recover() {
    let ledger = prepared_ledger();
    let mut ingress = PrivateCompactionIngress::new(
        ledger.clone(),
        AgentChatCompactionRecoveryAuthority::Observer,
    );
    assert_eq!(
        ingress.record(failed()).unwrap(),
        PrivateCompactionResult::DeniedObserver
    );
    assert!(ledger.find_event("compaction-failed-1").unwrap().is_none());
    assert_eq!(
        ledger
            .list_conversation_runs("conversation-a")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn an_unowned_compaction_fact_never_reaches_the_ledger() {
    let ledger = prepared_ledger();
    let mut ingress = PrivateCompactionIngress::new(
        ledger.clone(),
        AgentChatCompactionRecoveryAuthority::Approved,
    );
    let mut request = failed();
    request.coordinator_id = "daemon-b".into();
    assert!(ingress.record(request).is_err());
    assert!(ledger.find_event("compaction-failed-1").unwrap().is_none());
}

#[test]
fn a_selection_not_owned_by_the_durable_run_never_reaches_the_ledger() {
    let ledger = prepared_ledger();
    let mut ingress = PrivateCompactionIngress::new(
        ledger.clone(),
        AgentChatCompactionRecoveryAuthority::Approved,
    );
    let mut request = failed();
    request.selection.provider = AgentChatProvider::Claude;
    request.selection.model = "claude-sonnet".into();
    assert!(ingress.record(request).is_err());
    assert!(ledger.find_event("compaction-failed-1").unwrap().is_none());
}

#[test]
fn codex_normalized_observation_reaches_the_private_ingress_with_daemon_ids() {
    let ledger = prepared_ledger();
    let mut ingress = PrivateCompactionIngress::new(
        ledger.clone(),
        AgentChatCompactionRecoveryAuthority::Approved,
    );
    let facts = normalize_public_frame(
        PublicProvider::Codex,
        &serde_json::json!({
            "method": "item/started",
            "params": { "item": { "type": "contextCompaction", "secret": "ignored" } }
        }),
    );
    let [PublicWireFact::Compaction(observation)] = facts.as_slice() else {
        panic!("documented Codex compaction must normalize without raw detail");
    };
    assert_eq!(
        ingress.record_observation(observed(*observation)).unwrap(),
        PrivateCompactionResult::Recorded(gent_core::AgentChatCompactionEffect::None)
    );
    let source = ledger.find_event("compaction-observed-1").unwrap().unwrap();
    assert!(!source.payload.to_string().contains("secret"));
}

#[test]
fn a_typed_too_few_groups_observation_recovers_only_through_the_private_ingress() {
    let ledger = prepared_ledger();
    let mut ingress = PrivateCompactionIngress::new(
        ledger.clone(),
        AgentChatCompactionRecoveryAuthority::Approved,
    );
    let mut started = observed(PublicCompactionObservation::Started);
    started.event_id = "compaction-observed-started".into();
    assert_eq!(
        ingress.record_observation(started).unwrap(),
        PrivateCompactionResult::Recorded(gent_core::AgentChatCompactionEffect::None)
    );
    let mut failed = observed(PublicCompactionObservation::Failed {
        failure: AgentChatCompactionFailure::TooFewGroups,
    });
    failed.event_id = "compaction-observed-failed".into();
    let PrivateCompactionResult::Recovered(child) = ingress.record_observation(failed).unwrap()
    else {
        panic!("only the private ingress may reserve the recovery child");
    };
    assert_eq!(child.context_through_ordinal, 0);
    assert!(
        ledger
            .find_run_session_binding(&child.run_id.0)
            .unwrap()
            .is_none()
    );
}

#[test]
fn claude_result_text_never_fabricates_a_compaction_observation() {
    let facts = normalize_public_frame(
        PublicProvider::Claude,
        &serde_json::json!({
            "type": "result",
            "is_error": true,
            "result": "Couldn't compact: too_few_groups"
        }),
    );
    assert!(
        !facts
            .iter()
            .any(|fact| matches!(fact, PublicWireFact::Compaction(_)))
    );
}
