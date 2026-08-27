//! Private Claurst receives active goals only after Gent resolves durable ownership.

use gent_ports::{
    AgentChatLedger, ClaurstSessionBinding, ClaurstSourceId, ClaurstStartRequest,
    ClaurstSubmitRequest, GoalLedger, Ledger, RunLease,
};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_testkit::FakePrivateClaurstBridge;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRunId, AgentChatSelection, CapabilitySet,
    FrozenConversationContext, GOAL_SCHEMA_VERSION, GoalBinding, GoalRecord, GoalStatus, HostEpoch,
    ReceiptId,
};

use crate::private_claurst_ingress::PrivateClaurstIngress;

#[tokio::test]
async fn start_and_follow_up_replace_incoming_goals_with_the_active_gent_projection() {
    let ledger = ledger();
    ledger.create_goal(&goal()).unwrap();
    let bridge = FakePrivateClaurstBridge::default();
    bridge.push_start_binding(binding());
    let mut ingress = PrivateClaurstIngress::new(
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger,
        bridge.clone(),
        "daemon-a".into(),
    );
    let binding = ingress.start(start(), HostEpoch(1)).await.unwrap();
    ingress.submit(submit(binding)).await.unwrap();
    let starts = bridge.starts();
    let projected = starts[0].goal.as_ref().unwrap();
    assert_eq!(projected.goal.revision(), 1);
    assert_eq!(projected.goal.binding().goal_id, "goal-1");
    assert_eq!(projected.source_id.0, "source-a");
    let submissions = bridge.submissions();
    assert_eq!(submissions[0].goal.as_ref(), Some(projected));
}

#[tokio::test]
async fn private_claurst_rejects_an_incoming_goal_before_bridge_start() {
    let ledger = ledger();
    let bridge = FakePrivateClaurstBridge::default();
    let mut ingress = PrivateClaurstIngress::new(
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger,
        bridge.clone(),
        "daemon-a".into(),
    );
    let mut request = start();
    request.goal = Some(gent_ports::ClaurstGoalProjection {
        run_id: "run-a".into(),
        source_id: ClaurstSourceId("source-a".into()),
        goal: gent_types::GoalProjection::from_active(&goal()).unwrap(),
    });
    assert!(ingress.start(request, HostEpoch(1)).await.is_err());
    assert!(bridge.starts().is_empty());
}

fn ledger() -> SqliteLedger {
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
                model: "private".into(),
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

fn goal() -> GoalRecord {
    GoalRecord {
        schema_version: GOAL_SCHEMA_VERSION,
        binding: GoalBinding {
            goal_id: "goal-1".into(),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            run_id: AgentChatRunId("run-a".into()),
        },
        revision: 1,
        status: GoalStatus::Active,
        summary: "Complete safely".into(),
    }
}

fn start() -> ClaurstStartRequest {
    ClaurstStartRequest {
        run_id: "run-a".into(),
        source_id: ClaurstSourceId("source-a".into()),
        turn_id: "turn-a".into(),
        prompt: "start".into(),
        context: FrozenConversationContext::cleared(AgentChatConversationId(
            "conversation-a".into(),
        )),
        attachments: vec![],
        goal: None,
    }
}

fn binding() -> ClaurstSessionBinding {
    ClaurstSessionBinding {
        run_id: "run-a".into(),
        source_id: ClaurstSourceId("source-a".into()),
        opaque_session_id: "private-session".into(),
    }
}

fn submit(binding: ClaurstSessionBinding) -> ClaurstSubmitRequest {
    ClaurstSubmitRequest {
        binding,
        turn_id: "turn-b".into(),
        prompt: "continue".into(),
        attachments: vec![],
        goal: None,
    }
}
