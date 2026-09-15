use gent_core::{GoalDraft, GoalUserCommand, apply_user_command, create_goal, replaced_goal};
use gent_ports::{
    AgentChatProjectionLedger, AgentChatPromptDispatchLedger, AgentChatPromptLedger,
    AgentChatReadLedger, AgentChatWorkspaceLedger, ConversationActivityLedger, GoalLedger,
    GoalWrite, Ledger,
};
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatPromptOrigin, AgentChatPromptSaved,
    AgentChatProvider, AgentChatRequestId, AgentChatRunId, AgentChatSelection,
    ConversationActivityFact, ConversationActivityScope, GoalDispatchState, GoalRecord, GoalStatus,
    HostEpoch, ReceiptId, ToolActivity, ToolPhase, WorkspaceRecord,
};
use rusqlite::params;

use super::SqliteLedger;

fn seed(ledger: &SqliteLedger) {
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create-receipt".into()),
                idempotency_key: "create-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation-1".into()),
                run_id: AgentChatRunId("run-1".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Codex,
                    model: "gpt-5.6".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
}

fn ledger() -> SqliteLedger {
    let ledger = SqliteLedger::in_memory().unwrap();
    seed(&ledger);
    ledger
}

fn goal(id: &str, now: u64) -> GoalRecord {
    create_goal(
        GoalDraft {
            goal_id: id.into(),
            conversation_id: AgentChatConversationId("conversation-1".into()),
            objective: format!("Complete {id}"),
            token_budget: None,
            accounted_through_ordinal: 0,
        },
        now,
    )
    .unwrap()
}

fn create(request_id: &str) -> AgentChatPromptCreate {
    AgentChatPromptCreate {
        request_id: AgentChatRequestId(request_id.into()),
        receipt_id: ReceiptId(format!("receipt-{request_id}")),
        host_epoch: HostEpoch(1),
        conversation_id: AgentChatConversationId("conversation-1".into()),
        disposition: AgentChatPromptDisposition::Send,
        text: "work".into(),
        attachment_ids: vec![],
        tool_source_ids: vec![],
    }
}

fn prompt(ledger: &SqliteLedger, request_id: &str) -> AgentChatPromptSaved {
    ledger.save_agent_chat_prompt(&create(request_id)).unwrap()
}

fn continuation_origin() -> AgentChatPromptOrigin {
    AgentChatPromptOrigin::GoalContinuation {
        goal_id: "goal-1".into(),
        continues_after_ordinal: 1,
    }
}

fn tool(tool_use_id: &str, phase: ToolPhase) -> ToolActivity {
    ToolActivity {
        tool_use_id: tool_use_id.into(),
        tool_name: "Bash".into(),
        phase,
        output_digest: None,
        parent_tool_use_id: None,
    }
}

#[test]
fn every_goal_revision_is_durable_and_published_as_one_activity_fact() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.sqlite");
    let first = goal("goal-1", 10);
    {
        let ledger = SqliteLedger::open(&path).unwrap();
        seed(&ledger);
        assert_eq!(
            ledger
                .create_goal(None, None, &first, HostEpoch(1))
                .unwrap(),
            GoalWrite::Updated(first.clone())
        );
        assert_eq!(ledger.active_goals().unwrap(), vec![first.clone()]);
        ledger.close_ingress(HostEpoch(1)).unwrap();
        ledger.fence_and_open(HostEpoch(1)).unwrap();
    }
    let reopened = SqliteLedger::open(&path).unwrap();
    assert_eq!(
        reopened.current_goal("conversation-1").unwrap(),
        Some(first.clone())
    );
    let paused = apply_user_command(&first, 1, GoalUserCommand::Pause, 20).unwrap();
    assert_eq!(
        reopened
            .replace_goal(&first, &paused, HostEpoch(2))
            .unwrap(),
        GoalWrite::Updated(paused.clone())
    );
    assert!(reopened.active_goals().unwrap().is_empty());
    let facts = reopened
        .agent_chat_projection_page(&AgentChatConversationId("conversation-1".into()), 0, 100)
        .unwrap()
        .events
        .into_iter()
        .filter(|event| event.payload["activity"]["type"] == "goalUpdated")
        .map(|event| event.payload["activity"]["goal"]["status"].clone())
        .collect::<Vec<_>>();
    assert_eq!(facts, ["active", "paused"]);
}

#[test]
fn replacements_are_compare_and_swap_on_the_exact_revision() {
    let ledger = ledger();
    let first = goal("goal-1", 10);
    ledger
        .create_goal(None, None, &first, HostEpoch(1))
        .unwrap();
    let paused = apply_user_command(&first, 1, GoalUserCommand::Pause, 20).unwrap();
    ledger.replace_goal(&first, &paused, HostEpoch(1)).unwrap();
    let cleared = apply_user_command(&first, 1, GoalUserCommand::Clear, 30).unwrap();
    assert_eq!(
        ledger.replace_goal(&first, &cleared, HostEpoch(1)).unwrap(),
        GoalWrite::Current(Some(paused.clone()))
    );
    let mut altered = apply_user_command(&paused, 2, GoalUserCommand::Clear, 30).unwrap();
    altered.objective = "Different".into();
    assert!(
        ledger
            .replace_goal(&paused, &altered, HostEpoch(1))
            .is_err()
    );
    assert!(
        ledger
            .replace_goal(&paused, &cleared, HostEpoch(9))
            .is_err()
    );
}

#[test]
fn a_new_goal_atomically_clears_its_unsettled_predecessor() {
    let ledger = ledger();
    let first = goal("goal-1", 10);
    ledger
        .create_goal(None, None, &first, HostEpoch(1))
        .unwrap();
    let second = goal("goal-2", 20);
    assert_eq!(
        ledger
            .create_goal(None, None, &second, HostEpoch(1))
            .unwrap(),
        GoalWrite::Current(Some(first.clone()))
    );
    let replaced = replaced_goal(&first, 20).unwrap();
    ledger
        .create_goal(Some(&first), Some(&replaced), &second, HostEpoch(1))
        .unwrap();
    assert_eq!(
        ledger.find_goal("goal-1").unwrap().unwrap().status,
        GoalStatus::Cleared
    );
    assert_eq!(ledger.current_goal("conversation-1").unwrap(), Some(second));
}

#[test]
fn turn_observations_expose_continuations_dispatch_and_usage() {
    let ledger = ledger();
    let user = prompt(&ledger, "request-1");
    let continuation = ledger
        .save_agent_chat_prompt_with_origin(&create("goal-request"), &continuation_origin())
        .unwrap();
    ledger
        .release_agent_chat_prompt_after_readiness(
            &user.message.message_id,
            &user.run_id,
            HostEpoch(1),
        )
        .unwrap();
    {
        let connection = ledger.lock().unwrap();
        connection
            .execute(
                "UPDATE turns SET phase = 'completed' WHERE turn_id = ?1",
                [&user.message.turn_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO agent_chat_projection_events (conversation_id, source_event_id, kind, payload) VALUES ('conversation-1', 'failure-1', 'lifecycle', ?1)",
                params![serde_json::json!({"turnId": user.message.turn_id, "lifecycle": {"type": "event", "event": {"type": "providerFailure", "classification": "rateLimited"}}}).to_string()],
            )
            .unwrap();
    }
    let scope = |cursor| ConversationActivityScope {
        conversation_id: "conversation-1".into(),
        run_id: "run-1".into(),
        turn_id: user.message.turn_id.clone(),
        host_epoch: HostEpoch(1),
        cursor,
    };
    for fact in [
        ConversationActivityFact::ContextUsage {
            scope: scope(901),
            used_tokens: 120,
            window_tokens: None,
        },
        ConversationActivityFact::ContextUsage {
            scope: scope(902),
            used_tokens: 80,
            window_tokens: Some(200_000),
        },
        ConversationActivityFact::TokenUsage {
            scope: scope(905),
            usage: gent_types::TokenUsage {
                input_tokens: 18,
                output_tokens: 155,
                cache_read_tokens: 35_894,
                cache_creation_tokens: 8_843,
            },
        },
        ConversationActivityFact::ToolActivity {
            scope: scope(903),
            activity: tool("tool-a", ToolPhase::Started),
        },
        ConversationActivityFact::ToolActivity {
            scope: scope(904),
            activity: tool("tool-a", ToolPhase::Completed),
        },
    ] {
        ledger.append_conversation_activity(&fact).unwrap();
    }
    let turns = ledger.goal_turns("conversation-1", 0).unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].dispatch, GoalDispatchState::Pending);
    assert_eq!((turns[0].tokens, turns[0].tool_calls), (44_910, 1));
    assert_eq!(
        turns[0].failure,
        Some(gent_types::ProviderFailureClassification::RateLimited)
    );
    assert_eq!(turns[0].continuation_of, None);
    assert_eq!(turns[1].continuation_of.as_deref(), Some("goal-1"));
    assert_eq!(turns[1].dispatch, GoalDispatchState::AwaitingReadiness);
    assert!(!turns[1].held);
    assert_eq!(ledger.latest_turn_ordinal("conversation-1").unwrap(), 2);
    assert_eq!(
        ledger.goal_turns("conversation-1", 1).unwrap()[0].message_id,
        continuation.message.message_id
    );
}

#[test]
fn every_prompt_carries_its_typed_origin_on_live_and_restored_transcript_rows() {
    let ledger = ledger();
    let user = prompt(&ledger, "request-1");
    let continuation = ledger
        .save_agent_chat_prompt_with_origin(&create("goal-request"), &continuation_origin())
        .unwrap();
    let page = ledger
        .read_agent_chat_transcript("conversation-1", None, 10)
        .unwrap();
    let origins = page
        .events
        .iter()
        .map(|event| (event.event_id.clone(), event.origin.clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        origins,
        [
            (
                format!("user:{}", user.message.message_id),
                Some(AgentChatPromptOrigin::User)
            ),
            (
                format!("user:{}", continuation.message.message_id),
                Some(continuation_origin())
            ),
        ]
    );
    let projection = ledger
        .agent_chat_projection_page(&AgentChatConversationId("conversation-1".into()), 0, 100)
        .unwrap()
        .events
        .into_iter()
        .filter(|event| event.kind == "transcript")
        .map(|event| event.payload["origin"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        projection,
        [
            serde_json::json!({"kind": "user"}),
            serde_json::json!({"kind": "goalContinuation", "goalId": "goal-1", "continuesAfterOrdinal": 1}),
        ]
    );
}
