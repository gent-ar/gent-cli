use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use gent_drivers::codex_runner::CodexRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{ActiveGoalResolver, AgentChatLedger, AgentChatPromptLedger, LedgerError};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatPromptCreate,
    AgentChatPromptDisposition, AgentChatRequestId, AgentChatRunId, CapabilitySet,
    GOAL_SCHEMA_VERSION, GoalBinding, GoalProjection, GoalRecord, GoalStatus, HostEpoch, ReceiptId,
    TurnPhase,
};

use crate::approved_codex_host::ApprovedCodexHost;
use crate::codex_prompt_lifecycle_tests::{Resolver, Runner, compatibility, profile, selection};
use crate::public_driver_runtime::PublicDriversRuntime;

#[derive(Debug)]
struct FreshGoals(Mutex<VecDeque<GoalProjection>>);

impl ActiveGoalResolver for FreshGoals {
    fn resolve_active_goal(&self, _: &str, _: &str) -> Result<Option<GoalProjection>, LedgerError> {
        Ok(self.0.lock().unwrap().pop_front())
    }
}

fn projection(revision: u64) -> GoalProjection {
    GoalProjection::from_active(&GoalRecord {
        schema_version: GOAL_SCHEMA_VERSION,
        binding: GoalBinding {
            goal_id: "goal-1".into(),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            run_id: AgentChatRunId("run-a".into()),
        },
        revision,
        status: GoalStatus::Active,
        summary: "Finish without stopping".into(),
    })
    .unwrap()
}

fn save(ledger: &SqliteLedger, request: &str, text: &str) {
    ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(request.into()),
            receipt_id: ReceiptId(format!("receipt-{request}")),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            disposition: AgentChatPromptDisposition::Send,
            text: text.into(),
        })
        .unwrap();
}

#[test]
fn codex_resolves_a_fresh_goal_projection_for_initial_and_follow_up_turns() {
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
    save(&ledger, "first", "first prompt");
    let runner = Runner::default();
    let compatibility = compatibility();
    let runtime = PublicDriversRuntime::new(
        profile(&compatibility),
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger.clone(),
        compatibility,
        runner.clone(),
        Resolver,
    )
    .unwrap()
    .with_active_goal_resolver(Arc::new(FreshGoals(Mutex::new(VecDeque::from([
        projection(1),
        projection(4),
    ])))));
    let mut host = ApprovedCodexHost::new(runtime, "daemon-a".into(), None, HostEpoch(1), 1);
    host.tick().unwrap();
    assert_eq!(
        runner.state.lock().unwrap().prepared_goals[0]
            .as_ref()
            .unwrap()
            .revision(),
        1
    );
    runner
        .state
        .lock()
        .unwrap()
        .effects
        .push_back(vec![CodexRunnerEffect::Fact(PublicWireFact::Lifecycle(
            gent_types::NormalizedLifecycleSignal::RootPhase {
                phase: TurnPhase::Ready,
            },
        ))]);
    host.tick().unwrap();
    save(&ledger, "follow-up", "follow up");
    host.tick().unwrap();
    let state = runner.state.lock().unwrap();
    assert_eq!(state.submitted, ["follow up"]);
    assert_eq!(state.submitted_goals[0].as_ref().unwrap().revision(), 4);
}
