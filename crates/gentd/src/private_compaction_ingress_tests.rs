use gent_ports::{AgentChatLedger, ConversationLedger, Ledger, RunLease, RunSessionBinding};
use gent_runtime::AgentChatCompactionRecoveryAuthority;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatCompactionFact, AgentChatCompactionFailure, AgentChatConversationCreate,
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRunId,
    AgentChatSelection, HostEpoch, ReceiptId,
};

use crate::private_compaction_ingress::{
    PrivateCompactionIngress, PrivateCompactionRequest, PrivateCompactionResult,
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
