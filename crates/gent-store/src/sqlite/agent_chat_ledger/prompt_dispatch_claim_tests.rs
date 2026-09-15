use gent_core::{GoalDraft, GoalUserCommand, apply_user_command, create_goal};
use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatPromptLedger, AgentChatWorkspaceLedger,
    ConversationActivityLedger, GoalLedger,
};
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatPromptOrigin, AgentChatProvider,
    AgentChatRequestId, AgentChatRunId, AgentChatSelection, ConversationActivityFact, HostEpoch,
    ReceiptId, WorkspaceRecord,
};

use crate::SqliteLedger;

const EPOCH: HostEpoch = HostEpoch(1);

fn continuation(ledger: &SqliteLedger) -> gent_types::AgentChatPromptSaved {
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create".into()),
                idempotency_key: "create".into(),
                host_epoch: EPOCH,
                conversation_id: AgentChatConversationId("conversation-1".into()),
                run_id: AgentChatRunId("run-1".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "haiku".into(),
                    effort: AgentChatEffort::Low,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    let saved = ledger
        .save_agent_chat_prompt_with_origin(
            &AgentChatPromptCreate {
                request_id: AgentChatRequestId("goal-continuation".into()),
                receipt_id: ReceiptId("goal-continuation".into()),
                host_epoch: EPOCH,
                conversation_id: AgentChatConversationId("conversation-1".into()),
                disposition: AgentChatPromptDisposition::Send,
                text: "Continue".into(),
                attachment_ids: vec![],
                tool_source_ids: vec![],
            },
            &AgentChatPromptOrigin::GoalContinuation {
                goal_id: "goal-1".into(),
                continues_after_ordinal: 0,
            },
        )
        .unwrap();
    ledger
        .release_agent_chat_prompt_after_readiness(&saved.message.message_id, &saved.run_id, EPOCH)
        .unwrap();
    saved
}

fn goal(ledger: &SqliteLedger) -> gent_types::GoalRecord {
    let goal = create_goal(
        GoalDraft {
            goal_id: "goal-1".into(),
            conversation_id: AgentChatConversationId("conversation-1".into()),
            objective: "Finish".into(),
            token_budget: None,
            accounted_through_ordinal: 0,
        },
        10,
    )
    .unwrap();
    ledger.create_goal(None, None, &goal, EPOCH).unwrap();
    goal
}

#[test]
fn a_paused_goal_never_starts_its_already_admitted_continuation() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let saved = continuation(&ledger);
    let active = goal(&ledger);
    let paused = apply_user_command(&active, 1, GoalUserCommand::Pause, 20).unwrap();
    ledger.replace_goal(&active, &paused, EPOCH).unwrap();
    assert_eq!(
        ledger
            .claim_agent_chat_prompt_dispatch("gentd-1", EPOCH, AgentChatProvider::Claude)
            .unwrap(),
        None
    );
    let (phase, dispatch): (String, String) = ledger
        .lock()
        .unwrap()
        .query_row(
            "SELECT t.phase, d.state FROM turns t JOIN agent_chat_prompt_dispatches d ON d.message_id = ?1 WHERE t.turn_id = ?2",
            [&saved.message.message_id, &saved.message.turn_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (phase.as_str(), dispatch.as_str()),
        ("cancelled", "settled")
    );
    assert!(
        ledger
            .read_conversation_activity_page("conversation-1", "run-1", 0, 64)
            .unwrap()
            .facts
            .iter()
            .any(|fact| matches!(fact, ConversationActivityFact::PromptCanceled { message_id, .. } if *message_id == saved.message.message_id))
    );
}

#[test]
fn an_active_goal_continuation_is_claimed_normally() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let saved = continuation(&ledger);
    goal(&ledger);
    let claimed = ledger
        .claim_agent_chat_prompt_dispatch("gentd-1", EPOCH, AgentChatProvider::Claude)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.message.message_id, saved.message.message_id);
}
