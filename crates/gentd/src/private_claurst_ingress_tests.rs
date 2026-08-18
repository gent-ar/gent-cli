use gent_ports::{
    AgentChatLedger, ClaurstCheckpoint, ClaurstDrainBatch, ClaurstFactValue, ClaurstNormalizedFact,
    ClaurstSessionBinding, ClaurstSourceId, Ledger, RunCheckpointLedger, RunLease,
};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_testkit::FakePrivateClaurstBridge;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRunId, AgentChatSelection, CapabilitySet, HostEpoch,
    NormalizedLifecycleSignal, ReceiptId, RootActivity,
};

use crate::private_claurst_ingress::PrivateClaurstIngress;

#[tokio::test]
async fn drain_persists_normalized_facts_then_a_terminal_checkpoint() {
    let ledger = prepared_ledger();
    let bridge = FakePrivateClaurstBridge::default();
    let binding = binding();
    bridge.push_batch(ClaurstDrainBatch {
        facts: vec![fact(1)],
        checkpoint: Some(checkpoint(1)),
        session_binding: Some(binding.clone()),
        terminal: Some(gent_ports::ClaurstTerminal::Completed),
    });
    let mut ingress = PrivateClaurstIngress::new(
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger.clone(),
        bridge,
        "daemon-a".into(),
    );
    ingress.bind(binding.clone(), HostEpoch(1)).await.unwrap();
    assert_eq!(
        ingress
            .drain(&binding.source_id, HostEpoch(1))
            .await
            .unwrap(),
        crate::private_claurst_ingress::PrivateClaurstDrain {
            facts: 1,
            terminal: true,
        }
    );
    assert!(
        ledger
            .find_event(
                "claurst:0f9f5ce47831e099e77e295ed8bb627f089efa8672ee6fbdc49eac6f0d7f5275:fact-1"
            )
            .unwrap()
            .is_some()
    );
    assert_eq!(ledger.list_run_checkpoints("run-a").unwrap().len(), 1);
    assert!(
        ingress
            .drain(&binding.source_id, HostEpoch(1))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn rejects_out_of_order_or_rebound_batches_before_any_fact_is_persisted() {
    let ledger = prepared_ledger();
    let bridge = FakePrivateClaurstBridge::default();
    let binding = binding();
    bridge.push_batch(ClaurstDrainBatch {
        facts: vec![fact(2), fact(1)],
        checkpoint: Some(checkpoint(1)),
        session_binding: None,
        terminal: None,
    });
    let mut ingress = PrivateClaurstIngress::new(
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger.clone(),
        bridge,
        "daemon-a".into(),
    );
    ingress.bind(binding.clone(), HostEpoch(1)).await.unwrap();
    assert!(
        ingress
            .drain(&binding.source_id, HostEpoch(1))
            .await
            .is_err()
    );
    assert!(
        ledger
            .find_event(
                "claurst:0f9f5ce47831e099e77e295ed8bb627f089efa8672ee6fbdc49eac6f0d7f5275:fact-1"
            )
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn rejects_session_rebinding_and_unsealed_terminal_batches() {
    let ledger = prepared_ledger();
    let bridge = FakePrivateClaurstBridge::default();
    let binding = binding();
    bridge.push_batch(ClaurstDrainBatch {
        facts: vec![fact(1)],
        checkpoint: Some(checkpoint(1)),
        session_binding: Some(ClaurstSessionBinding {
            opaque_session_id: "changed".into(),
            ..binding.clone()
        }),
        terminal: None,
    });
    bridge.push_batch(ClaurstDrainBatch {
        facts: Vec::new(),
        checkpoint: None,
        session_binding: None,
        terminal: Some(gent_ports::ClaurstTerminal::Interrupted),
    });
    let mut ingress = PrivateClaurstIngress::new(
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger,
        bridge,
        "daemon-a".into(),
    );
    ingress.bind(binding.clone(), HostEpoch(1)).await.unwrap();
    assert!(
        ingress
            .drain(&binding.source_id, HostEpoch(1))
            .await
            .is_err()
    );
    assert!(
        ingress
            .drain(&binding.source_id, HostEpoch(1))
            .await
            .is_err()
    );
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
            selection: AgentChatSelection {
                provider: AgentChatProvider::Claurst,
                model: "claurst-private".into(),
                effort: AgentChatEffort::Medium,
                mode: AgentChatMode::Agent,
            },
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
}

fn binding() -> ClaurstSessionBinding {
    ClaurstSessionBinding {
        run_id: "run-a".into(),
        source_id: ClaurstSourceId("source-a".into()),
        opaque_session_id: "private-session".into(),
    }
}

fn checkpoint(cursor: u64) -> ClaurstCheckpoint {
    ClaurstCheckpoint {
        run_id: "run-a".into(),
        source_id: ClaurstSourceId("source-a".into()),
        cursor,
        state_digest_sha256: "d".repeat(64),
    }
}

fn fact(cursor: u64) -> ClaurstNormalizedFact {
    ClaurstNormalizedFact {
        source_id: ClaurstSourceId("source-a".into()),
        cursor,
        value: ClaurstFactValue::Lifecycle(NormalizedLifecycleSignal::RootActivity {
            activity: RootActivity::Generating,
        }),
    }
}
