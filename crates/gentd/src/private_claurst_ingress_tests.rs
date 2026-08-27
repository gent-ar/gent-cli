use gent_ports::{
    AgentChatWorkspaceLedger, ClaurstCheckpoint, ClaurstDrainBatch, ClaurstFactValue,
    ClaurstNormalizedFact, ClaurstSessionBinding, ClaurstSourceId, Ledger, RunCheckpointLedger,
    RunLease,
};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_testkit::FakePrivateClaurstBridge;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRunId, AgentChatSelection, CapabilitySet,
    FrozenConversationContext, HostEpoch, NormalizedLifecycleSignal, ReceiptId, RootActivity,
    WorkspaceRecord,
};

use crate::private_claurst_ingress::PrivateClaurstIngress;

#[tokio::test]
async fn start_accepts_only_an_exact_daemon_owned_private_session() {
    let ledger = prepared_ledger();
    let bridge = FakePrivateClaurstBridge::default();
    let binding = binding();
    bridge.push_start_binding(binding.clone());
    let mut ingress = PrivateClaurstIngress::new(
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger.clone(),
        bridge,
        "daemon-a".into(),
    );
    assert_eq!(
        ingress.start(start_request(), HostEpoch(1)).await.unwrap(),
        binding
    );
    assert!(
        ledger
            .find_event(
                "claurst:0f9f5ce47831e099e77e295ed8bb627f089efa8672ee6fbdc49eac6f0d7f5275:session"
            )
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn follow_up_requires_the_exact_bound_private_session() {
    let ledger = prepared_ledger();
    let bridge = FakePrivateClaurstBridge::default();
    let binding = binding();
    bridge.push_start_binding(binding.clone());
    let mut ingress = PrivateClaurstIngress::new(
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger,
        bridge,
        "daemon-a".into(),
    );
    // The sidecar must be bound through start before it can receive a follow-up.
    ingress.start(start_request(), HostEpoch(1)).await.unwrap();
    ingress
        .submit(submit_request(binding.clone()))
        .await
        .unwrap();
    let mut other_binding = binding;
    other_binding.opaque_session_id = "other-session".into();
    assert!(ingress.submit(submit_request(other_binding)).await.is_err());
}

#[tokio::test]
async fn drain_persists_normalized_facts_then_a_terminal_checkpoint() {
    let ledger = prepared_ledger();
    let bridge = FakePrivateClaurstBridge::default();
    let binding = binding();
    bridge.push_batch(ClaurstDrainBatch {
        facts: vec![fact(1)],
        permissions: vec![],
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
            terminal_phase: Some(gent_types::DurableTurnPhase::Completed),
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
        permissions: vec![],
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
        permissions: vec![],
        checkpoint: Some(checkpoint(1)),
        session_binding: Some(ClaurstSessionBinding {
            opaque_session_id: "changed".into(),
            ..binding.clone()
        }),
        terminal: None,
    });
    bridge.push_batch(ClaurstDrainBatch {
        facts: Vec::new(),
        permissions: vec![],
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

#[tokio::test]
async fn cancellation_reaches_only_the_exact_active_binding_without_settlement() {
    let ledger = prepared_ledger();
    let bridge = FakePrivateClaurstBridge::default();
    let binding = binding();
    bridge.push_start_binding(binding.clone());
    let mut ingress = PrivateClaurstIngress::new(
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger.clone(),
        bridge.clone(),
        "daemon-a".into(),
    );
    ingress.start(start_request(), HostEpoch(1)).await.unwrap();
    assert!(ingress.cancel_run("other-run").await.is_err());
    ingress.cancel_run("run-a").await.unwrap();
    assert_eq!(bridge.cancellations(), vec![binding]);
    assert!(ingress.cancel_run("run-a").await.is_err());
    assert!(ledger
        .find_event("claurst:0f9f5ce47831e099e77e295ed8bb627f089efa8672ee6fbdc49eac6f0d7f5275:interrupted")
        .unwrap()
        .is_none());
    assert!(ledger.list_run_checkpoints("run-a").unwrap().is_empty());
}

pub(crate) fn prepared_ledger() -> SqliteLedger {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
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
            },
            &WorkspaceRecord {
                workspace_id: "workspace-a".into(),
                canonical_path: "/workspace-a".into(),
            },
        )
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

pub(crate) fn binding() -> ClaurstSessionBinding {
    ClaurstSessionBinding {
        run_id: "run-a".into(),
        source_id: ClaurstSourceId("source-a".into()),
        opaque_session_id: "private-session".into(),
    }
}

pub(crate) fn start_request() -> gent_ports::ClaurstStartRequest {
    gent_ports::ClaurstStartRequest {
        run_id: "run-a".into(),
        source_id: ClaurstSourceId("source-a".into()),
        turn_id: "turn-a".into(),
        prompt: "continue".into(),
        context: FrozenConversationContext::cleared(AgentChatConversationId(
            "conversation-a".into(),
        )),
        attachments: vec![],
        goal: None,
    }
}

fn submit_request(binding: ClaurstSessionBinding) -> gent_ports::ClaurstSubmitRequest {
    gent_ports::ClaurstSubmitRequest {
        binding,
        turn_id: "turn-b".into(),
        prompt: "continue".into(),
        attachments: vec![],
        goal: None,
    }
}

pub(crate) fn checkpoint(cursor: u64) -> ClaurstCheckpoint {
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
