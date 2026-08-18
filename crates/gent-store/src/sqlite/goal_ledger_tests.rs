use gent_ports::{AgentChatLedger, GoalLedger, GoalWrite, Ledger};
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRunId, AgentChatSelection, GOAL_SCHEMA_VERSION, GoalBinding,
    GoalRecord, GoalStatus, HostEpoch, ReceiptId,
};

use super::SqliteLedger;

fn ledger() -> SqliteLedger {
    let ledger = SqliteLedger::in_memory().unwrap();
    seed(&ledger);
    ledger
}

fn seed(ledger: &SqliteLedger) {
    ledger
        .create_agent_chat_conversation(&AgentChatConversationCreate {
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
        })
        .unwrap();
}

fn goal(id: &str) -> GoalRecord {
    GoalRecord {
        schema_version: GOAL_SCHEMA_VERSION,
        binding: GoalBinding {
            goal_id: id.into(),
            conversation_id: AgentChatConversationId("conversation-1".into()),
            run_id: AgentChatRunId("run-1".into()),
        },
        revision: 1,
        status: GoalStatus::Active,
        summary: format!("Complete {id}"),
    }
}

#[test]
fn goals_are_durable_across_epoch_fences_and_keep_creation_order() {
    let ledger = ledger();
    let first = goal("goal-1");
    let second = goal("goal-2");
    assert_eq!(
        ledger.create_goal(&first).unwrap(),
        GoalWrite::Created(first.clone())
    );
    assert_eq!(
        ledger.create_goal(&first).unwrap(),
        GoalWrite::Current(first.clone())
    );
    assert_eq!(
        ledger.create_goal(&second).unwrap(),
        GoalWrite::Created(second.clone())
    );
    assert_eq!(
        ledger.close_ingress(HostEpoch(1)).unwrap().epoch,
        HostEpoch(1)
    );
    assert_eq!(
        ledger.fence_and_open(HostEpoch(1)).unwrap().epoch,
        HostEpoch(2)
    );
    assert_eq!(
        ledger.find_goal(&first.binding).unwrap(),
        Some(first.clone())
    );
    assert_eq!(
        ledger.conversation_goals("conversation-1").unwrap(),
        vec![first, second]
    );
}

#[test]
fn goals_survive_a_database_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.sqlite");
    let first = goal("goal-1");
    {
        let ledger = SqliteLedger::open(&path).unwrap();
        seed(&ledger);
        assert_eq!(
            ledger.create_goal(&first).unwrap(),
            GoalWrite::Created(first.clone())
        );
    }
    let reopened = SqliteLedger::open(&path).unwrap();
    assert_eq!(reopened.find_goal(&first.binding).unwrap(), Some(first));
}

#[test]
fn replacements_are_revision_checked_idempotent_and_reject_immutable_conflicts() {
    let ledger = ledger();
    let first = goal("goal-1");
    ledger.create_goal(&first).unwrap();
    let completed = GoalRecord {
        revision: 2,
        status: GoalStatus::Completed,
        ..first.clone()
    };
    assert_eq!(
        ledger.replace_goal(&first, &completed).unwrap(),
        GoalWrite::Updated(completed.clone())
    );
    assert_eq!(
        ledger.replace_goal(&first, &completed).unwrap(),
        GoalWrite::Current(completed.clone())
    );
    let stale = GoalRecord {
        revision: 2,
        status: GoalStatus::Abandoned,
        ..first.clone()
    };
    assert_eq!(
        ledger.replace_goal(&first, &stale).unwrap(),
        GoalWrite::Current(completed)
    );
    let mut altered = first.clone();
    altered.revision = 2;
    altered.binding.goal_id = "goal-2".into();
    assert!(ledger.replace_goal(&first, &altered).is_err());
    let conflicting_create = GoalRecord {
        summary: "Different".into(),
        ..first
    };
    assert!(ledger.create_goal(&conflicting_create).is_err());
}
