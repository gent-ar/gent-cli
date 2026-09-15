use gent_ports::{ConversationLedger, PolicyLedger, ReviewedPlanLedger};
use gent_runtime::{ReviewedPlanAuthority, ReviewedPlanResult, ReviewedPlanService};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatPromptDisposition::Send, AgentChatProvider,
    AgentChatRequestId, AgentChatSelection, ContextPolicy, DurableTurnPhase, PlanStatus,
    PolicyScope, ReceiptId, StartImplementationRequest,
};

use super::fake_cli::FakeClaudeDaemon;
use crate::goal_pursuit_host::test_support::RouterAdmission;

fn pursue(daemon: &mut FakeClaudeDaemon, plans: &ReviewedPlanService<SqliteLedger>) {
    let epoch = daemon.epoch;
    let failures = plans
        .pursue(
            epoch,
            &mut RouterAdmission {
                ledger: &daemon.ledger,
                router: &mut daemon.router,
                epoch,
            },
        )
        .unwrap();
    assert!(failures.is_empty(), "{failures:?}");
}

#[test]
fn a_claude_plan_mode_turn_becomes_a_reviewed_plan_that_starts_an_agent_implementation() {
    let mut daemon = FakeClaudeDaemon::start();
    let (conversation, planning_run) = daemon.planning_conversation("plan");
    let planning = daemon.prompt(&conversation, "PLAN a README", Send);
    daemon.drive_until("plan turn", |daemon| daemon.phase(&planning).is_terminal());
    assert_eq!(daemon.phase(&planning), DurableTurnPhase::Completed);
    let plan_launch = daemon.launches().last().unwrap().clone();
    assert!(
        plan_launch
            .windows(2)
            .any(|pair| pair == ["--permission-mode", "plan"])
    );
    assert!(
        !plan_launch
            .iter()
            .any(|argument| argument.contains("Plan Mode"))
    );

    let plans = ReviewedPlanService::new(daemon.ledger.clone(), ReviewedPlanAuthority::Approved);
    pursue(&mut daemon, &plans);
    let ReviewedPlanResult::Plan(Some(plan)) = plans.review(&conversation.0, None).unwrap() else {
        panic!("the plan-mode turn must produce a reviewable plan");
    };
    assert_eq!(
        (plan.status, plan.content.as_str(), &plan.source_run_id),
        (
            PlanStatus::ReadyForReview,
            "1. Create README.md\n2. Add a usage section",
            &planning_run
        )
    );
    let policy = daemon
        .ledger
        .current_policy("workspace-fake-claude", PolicyScope::ProviderPermissions)
        .unwrap()
        .unwrap();
    let ReviewedPlanResult::Started(started) = plans
        .start(&StartImplementationRequest {
            request_id: AgentChatRequestId("start-plan".into()),
            receipt_id: ReceiptId("start-plan".into()),
            idempotency_key: "start-plan".into(),
            host_epoch: daemon.epoch,
            policy_workspace_id: "workspace-fake-claude".into(),
            policy_revision: policy.revision,
            conversation_id: conversation.clone(),
            plan_id: plan.plan_id.clone(),
            plan_revision: plan.revision,
            plan_content_digest_sha256: plan.content_digest_sha256.clone(),
            parent_run_id: planning_run,
            selection: AgentChatSelection {
                provider: AgentChatProvider::Claude,
                model: "sonnet".into(),
                effort: AgentChatEffort::Medium,
                mode: AgentChatMode::Agent,
            },
            context_policy: ContextPolicy::Preserve,
        })
        .unwrap()
    else {
        panic!("approval must reserve the implementation run");
    };
    pursue(&mut daemon, &plans);
    pursue(&mut daemon, &plans);
    let implementation_run = started.implementation_run_id.0.clone();
    daemon.drive_until("implementation turn", |daemon| {
        let turns = daemon.ledger.list_run_turns(&implementation_run).unwrap();
        turns.len() == 1 && turns[0].phase.is_terminal()
    });
    assert!(
        daemon
            .ledger
            .implementations_awaiting_prompt()
            .unwrap()
            .is_empty()
    );
    let implementation_launch = daemon.launches().last().unwrap().clone();
    assert!(
        implementation_launch
            .windows(2)
            .any(|pair| pair == ["--permission-mode", "manual"])
    );
    let projection = daemon.projection(&conversation);
    assert!(projection.iter().any(|event| {
        event.payload["kind"] == "userMessage"
            && event.payload["runId"] == implementation_run.as_str()
            && event.payload["text"]
                .as_str()
                .is_some_and(|text| text.contains("2. Add a usage section"))
    }));
    assert!(projection.iter().any(|event| {
        event.payload["activity"]["type"] == "planUpdated"
            && event.payload["activity"]["plan"]["status"] == "approved"
    }));
}
