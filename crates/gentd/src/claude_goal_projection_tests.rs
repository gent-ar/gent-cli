//! Claude receives each goal only from the fresh Gent-owned active-goal resolver.

use std::sync::Arc;

use gent_ports::{
    ActiveGoalResolver, AgentChatPromptLedger, AgentChatWorkspaceLedger, LedgerError,
};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, CapabilitySet, GOAL_SCHEMA_VERSION, GoalBinding,
    GoalProjection, GoalRecord, GoalStatus, HostEpoch, ReceiptId, WorkspaceRecord,
};

use crate::approved_claude_host::ApprovedClaudeHost;
use crate::claude_prompt_lifecycle_tests::{Resolver, Runner, compatibility, profile};
use crate::public_driver_runtime::PublicDriversRuntime;

#[derive(Debug)]
struct Goal(GoalProjection);

impl ActiveGoalResolver for Goal {
    fn resolve_active_goal(
        &self,
        conversation_id: &str,
        run_id: &str,
    ) -> Result<Option<GoalProjection>, LedgerError> {
        (conversation_id == "conversation-a" && run_id == "run-a")
            .then(|| self.0.clone())
            .ok_or_else(|| LedgerError::Invariant("goal resolver received another run".into()))
            .map(Some)
    }
}

#[test]
fn claude_prepares_the_exact_gent_resolved_revisioned_goal() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = AgentChatConversationId("conversation-a".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation_id.clone(),
                run_id: AgentChatRunId("run-a".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "sonnet".into(),
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
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-1".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id,
            disposition: AgentChatPromptDisposition::Send,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: "continue".into(),
        })
        .unwrap();
    crate::readiness_test_support::release(&ledger, &saved);
    let goal = projection();
    let runner = Runner::default();
    let compatibility = compatibility();
    let runtime = PublicDriversRuntime::new(
        profile(&compatibility),
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger,
        compatibility,
        runner.clone(),
        Resolver,
    )
    .unwrap()
    .with_active_goal_resolver(Arc::new(Goal(goal.clone())));
    let mut host = ApprovedClaudeHost::new(runtime, "daemon-a".into(), HostEpoch(1), 1, None);
    host.tick().unwrap();
    assert_eq!(runner.0.lock().unwrap().prepared_goals, [Some(goal)]);
}

fn projection() -> GoalProjection {
    GoalProjection::from_active(&GoalRecord {
        schema_version: GOAL_SCHEMA_VERSION,
        binding: GoalBinding {
            goal_id: "goal-1".into(),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            run_id: AgentChatRunId("run-a".into()),
        },
        revision: 4,
        status: GoalStatus::Active,
        summary: "Finish safely".into(),
    })
    .unwrap()
}
