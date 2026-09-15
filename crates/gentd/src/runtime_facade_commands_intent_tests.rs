use gent_protocol::{AgentChatIntentFrame, agent_chat_commands::CommandOutcome};
use gent_types::{
    AgentChatCommandIntent, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRequestId,
    DurableTurnPhase, ReceiptId,
};

use super::{create, invoke, run_selection, runtime};
use crate::api::RuntimeApi;

fn applied(outcome: &CommandOutcome) -> (AgentChatCommandIntent, String, Option<String>) {
    match outcome {
        CommandOutcome::IntentApplied {
            intent,
            conversation_id,
            run_id,
        } => (
            *intent,
            conversation_id.0.clone(),
            run_id.as_ref().map(|run| run.0.clone()),
        ),
        other @ CommandOutcome::Delivered { .. } => panic!("{other:?}"),
    }
}

#[test]
fn selection_commands_switch_the_current_run_and_replay_their_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = runtime(directory.path());
    let conversation_id = create(&runtime, "select", AgentChatProvider::Codex, "gpt-5.6");
    let effort = invoke(&runtime, "effort", Some(&conversation_id), "effort", "HIGH").unwrap();
    assert_eq!(applied(&effort).0, AgentChatCommandIntent::SelectEffort);
    assert_eq!(
        invoke(&runtime, "effort", Some(&conversation_id), "effort", "HIGH").unwrap(),
        effort
    );
    assert_eq!(
        run_selection(&runtime, &conversation_id).effort,
        AgentChatEffort::High
    );
    invoke(&runtime, "plan", Some(&conversation_id), "plan", "").unwrap();
    assert_eq!(
        run_selection(&runtime, &conversation_id).mode,
        AgentChatMode::Plan
    );
    invoke(&runtime, "mode", Some(&conversation_id), "mode", "ask").unwrap();
    assert_eq!(
        run_selection(&runtime, &conversation_id).mode,
        AgentChatMode::Ask
    );
    invoke(
        &runtime,
        "model",
        Some(&conversation_id),
        "model",
        "gpt-5.6-luna",
    )
    .unwrap();
    let luna = run_selection(&runtime, &conversation_id);
    assert_eq!(
        (luna.model.as_str(), luna.effort),
        ("gpt-5.6-luna", AgentChatEffort::Medium)
    );
    let clear = invoke(&runtime, "clear", Some(&conversation_id), "reset", "").unwrap();
    assert_eq!(applied(&clear).0, AgentChatCommandIntent::ClearContext);
    invoke(
        &runtime,
        "provider",
        Some(&conversation_id),
        "provider",
        "claude",
    )
    .unwrap();
    let claude = run_selection(&runtime, &conversation_id);
    assert_eq!(
        (claude.provider, claude.model.as_str()),
        (AgentChatProvider::Claude, "default")
    );
}

#[test]
fn new_and_fork_commands_apply_their_gent_intents() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = runtime(directory.path());
    let conversation_id = create(&runtime, "intents", AgentChatProvider::Codex, "gpt-5.6");
    let created = applied(&invoke(&runtime, "new", Some(&conversation_id), "new", "").unwrap());
    assert_eq!(created.0, AgentChatCommandIntent::CreateConversation);
    assert_ne!(created.1, conversation_id.0);
    let from_home = applied(&invoke(&runtime, "new-home", None, "new", "").unwrap());
    assert_ne!(from_home.1, created.1);
    assert_eq!(
        invoke(&runtime, "fork-empty", Some(&conversation_id), "fork", "")
            .unwrap_err()
            .code,
        "commandArgumentsInvalid"
    );
    let accepted = runtime
        .agent_chat_intent(AgentChatIntentFrame::SendPrompt {
            request_id: AgentChatRequestId("prompt".into()),
            receipt_id: ReceiptId("prompt".into()),
            conversation_id: conversation_id.clone(),
            text: "hello".into(),
            attachment_ids: Vec::new(),
        })
        .unwrap();
    let [AgentChatIntentFrame::Accepted { turn_id, .. }] = accepted.as_slice() else {
        panic!("{accepted:?}");
    };
    assert_eq!(
        invoke(&runtime, "fork-busy", Some(&conversation_id), "fork", "")
            .unwrap_err()
            .code,
        "commandBlockedByActiveTurn"
    );
    runtime
        .coordinator
        .transition_turn(
            turn_id,
            DurableTurnPhase::Active,
            DurableTurnPhase::Completed,
        )
        .unwrap();
    let forked = applied(&invoke(&runtime, "fork", Some(&conversation_id), "fork", "").unwrap());
    assert_eq!(forked.0, AgentChatCommandIntent::ForkConversation);
    assert_ne!(forked.1, conversation_id.0);
}

#[test]
fn goal_command_sets_a_budgeted_goal_and_replays_by_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = runtime(directory.path());
    let conversation_id = create(&runtime, "goals", AgentChatProvider::Codex, "gpt-5.6");
    let goal = invoke(
        &runtime,
        "goal",
        Some(&conversation_id),
        "goal",
        "ship the release",
    )
    .unwrap();
    assert_eq!(
        applied(&goal),
        (
            AgentChatCommandIntent::Goal,
            conversation_id.0.clone(),
            None
        )
    );
    assert_eq!(
        invoke(
            &runtime,
            "goal",
            Some(&conversation_id),
            "goal",
            "ship the release"
        )
        .unwrap(),
        goal
    );
    assert_eq!(
        invoke(
            &runtime,
            "goal-budget-bad",
            Some(&conversation_id),
            "goal",
            "--budget lots ship"
        )
        .unwrap_err()
        .code,
        "commandArgumentsInvalid"
    );
    invoke(
        &runtime,
        "goal-budget",
        Some(&conversation_id),
        "goal",
        "--budget 1.5k ship with a budget",
    )
    .unwrap();
    let budgeted = runtime.goals.current(&conversation_id).unwrap();
    assert!(
        matches!(budgeted, gent_runtime::GoalResult::Goal(Some(goal)) if goal.token_budget == Some(1_500) && goal.objective == "ship with a budget")
    );
    invoke(
        &runtime,
        "goal-pause",
        Some(&conversation_id),
        "goal",
        "pause",
    )
    .unwrap();
    invoke(
        &runtime,
        "goal-pause",
        Some(&conversation_id),
        "goal",
        "pause",
    )
    .unwrap();
    assert_eq!(
        invoke(&runtime, "goal-empty", Some(&conversation_id), "goal", "")
            .unwrap_err()
            .code,
        "commandArgumentsInvalid"
    );
}
