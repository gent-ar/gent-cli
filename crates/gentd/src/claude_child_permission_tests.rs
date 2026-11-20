use gent_drivers::claude_control::ClaudePermissionRequest;
use gent_drivers::claude_runner::ClaudeRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{
    ConversationActivityLedger, ConversationLedger, PendingPermissionLedger, PolicyLedger,
};
use gent_types::{
    AgentChatConversationId, AgentChatRunId, ConversationActivityFact, DurableTurnPhase,
    NormalizedLifecycleSignal, NormalizedProviderEvent, TurnPhase, WorkPhase,
};

use super::{child_started, emit};
use crate::regression_suites::claude_warm_pool_tests::Pool;

fn asking_pool() -> (Pool, AgentChatConversationId, String) {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    pool.ledger
        .save_policy(&gent_types::PolicyRecord {
            policy_id: "policy-a".into(),
            workspace_id: "workspace-a".into(),
            scope: gent_types::PolicyScope::ProviderPermissions,
            revision: 1,
            mode: gent_types::PermissionMode::AskEveryTime,
            containment: gent_types::Containment::Unsandboxed,
            allowed_tools: vec![],
            allowed_categories: vec![],
            allowed_commands: Vec::new(),
        })
        .unwrap();
    let run_id = pool.send(&conversation, "a1");
    pool.finish(run_id.clone());
    emit(&mut pool, &run_id, vec![child_started("background")]);
    assert_eq!(pool.send(&conversation, "a2"), run_id);
    emit(
        &mut pool,
        &run_id,
        vec![ClaudeRunnerEffect::PermissionRequest(
            ClaudePermissionRequest {
                request_id: "child-ask".into(),
                tool_use_id: "tool-child".into(),
                tool_name: "write_file".into(),
                operation_preview: Some(serde_json::json!({"file_path": "/workspace-a/c.rs"})),
                child_id: Some("background".into()),
            },
        )],
    );
    emit(
        &mut pool,
        &run_id,
        vec![ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::RootPhase {
                phase: TurnPhase::Ready,
            },
        ))],
    );
    assert!(
        pool.ledger
            .list_run_turns(&run_id)
            .unwrap()
            .iter()
            .all(|turn| turn.phase == DurableTurnPhase::Completed)
    );
    (pool, conversation, run_id)
}

fn pending(pool: &Pool, conversation: &AgentChatConversationId, run_id: &str) -> Option<String> {
    pool.ledger
        .pending_permission(conversation, &AgentChatRunId(run_id.into()))
        .unwrap()
        .map(|pending| pending.binding.decision_id.0)
}

fn settled(pool: &Pool, conversation: &AgentChatConversationId, run_id: &str) -> usize {
    pool.ledger
        .read_conversation_activity_page(&conversation.0, run_id, 0, 50)
        .unwrap()
        .facts
        .iter()
        .filter(|fact| {
            matches!(fact, ConversationActivityFact::DecisionSettled { decision_id, .. } if decision_id == "child-ask")
        })
        .count()
}

#[test]
fn a_background_child_request_outlives_the_turn_that_was_open_when_it_arrived() {
    let (mut pool, conversation, run_id) = asking_pool();
    assert_eq!(
        pending(&pool, &conversation, &run_id).as_deref(),
        Some("child-ask")
    );
    assert_eq!(settled(&pool, &conversation, &run_id), 0);
    let decision_pending = pool
        .ledger
        .read_conversation_activity_page(&conversation.0, &run_id, 0, 50)
        .unwrap()
        .facts
        .into_iter()
        .find_map(|fact| match fact {
            ConversationActivityFact::DecisionPending { request, .. } => request,
            _ => None,
        })
        .unwrap();
    assert_eq!(
        decision_pending.request.child_id.as_deref(),
        Some("background")
    );

    emit(
        &mut pool,
        &run_id,
        vec![ClaudeRunnerEffect::Fact(PublicWireFact::Event(
            NormalizedProviderEvent::ChildTerminal {
                child_id: "background".into(),
                phase: WorkPhase::Done,
            },
        ))],
    );
    assert_eq!(pending(&pool, &conversation, &run_id), None);
    assert_eq!(settled(&pool, &conversation, &run_id), 1);
}

#[test]
fn claude_cancelling_a_background_child_request_retires_it_once() {
    let (mut pool, conversation, run_id) = asking_pool();
    emit(
        &mut pool,
        &run_id,
        vec![ClaudeRunnerEffect::PermissionCancelled {
            request_id: "child-ask".into(),
        }],
    );
    assert_eq!(pending(&pool, &conversation, &run_id), None);
    assert_eq!(settled(&pool, &conversation, &run_id), 1);
    emit(
        &mut pool,
        &run_id,
        vec![ClaudeRunnerEffect::Fact(PublicWireFact::Event(
            NormalizedProviderEvent::ChildTerminal {
                child_id: "background".into(),
                phase: WorkPhase::Done,
            },
        ))],
    );
    assert_eq!(settled(&pool, &conversation, &run_id), 1);
}
