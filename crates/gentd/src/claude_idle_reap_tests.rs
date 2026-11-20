use std::time::{Duration, Instant};

use gent_drivers::claude_control::ClaudePermissionRequest;
use gent_drivers::claude_runner::ClaudeRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{
    ConversationActivityLedger, ConversationLedger, PendingPermissionLedger, PolicyLedger,
};
use gent_types::{
    AgentChatRunId, ConversationActivityFact, NormalizedLifecycleSignal, NormalizedProviderEvent,
    TurnPhase, WorkPhase,
};

use super::claude_warm_pool_tests::Pool;
use crate::provider_warm_pool::WarmPoolRequest;

const SERVED_IDLE: Duration = Duration::from_secs(600);

fn reap(pool: &mut Pool, now: Instant) -> bool {
    pool.host
        .tend_warm_pool(&WarmPoolRequest { now, target: None })
        .is_ok()
}

fn emit(pool: &mut Pool, run_id: &str, effects: Vec<ClaudeRunnerEffect>) {
    pool.runner
        .0
        .lock()
        .unwrap()
        .effects
        .entry(run_id.into())
        .or_default()
        .push_back(effects);
    pool.host.tick().unwrap();
}

fn child_started(child_id: &str) -> ClaudeRunnerEffect {
    ClaudeRunnerEffect::Fact(PublicWireFact::Event(
        NormalizedProviderEvent::ChildStarted {
            child_id: child_id.into(),
            parent_tool_use_id: format!("agent-{child_id}"),
        },
    ))
}

fn child_output(child_id: &str) -> ClaudeRunnerEffect {
    ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::ChildPhase {
            child_id: child_id.into(),
            phase: WorkPhase::Running,
        },
    ))
}

fn releases(pool: &Pool) -> Vec<String> {
    pool.runner.0.lock().unwrap().releases.clone()
}

#[test]
fn a_settled_claude_process_is_reaped_once_it_has_been_idle_for_the_served_timeout() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    let run_id = pool.send(&conversation, "a1");
    pool.finish(run_id);
    let now = Instant::now();

    assert!(reap(&mut pool, now));
    assert!(reap(&mut pool, now + Duration::from_secs(599)));
    assert!(releases(&pool).is_empty());

    assert!(reap(&mut pool, now + SERVED_IDLE));
    assert_eq!(releases(&pool), ["run-a"]);
}

#[test]
fn a_claude_turn_that_is_still_running_is_never_reaped() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    pool.send(&conversation, "a1");
    let now = Instant::now();

    assert!(reap(&mut pool, now));
    assert!(reap(&mut pool, now + SERVED_IDLE * 100));

    assert!(releases(&pool).is_empty());
}

#[test]
fn background_subagents_keep_a_settled_process_alive_only_while_they_produce_output() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    let run_id = pool.send(&conversation, "a1");
    emit(
        &mut pool,
        &run_id,
        vec![
            ClaudeRunnerEffect::Fact(PublicWireFact::SessionStarted {
                provider_session_id: "session-run-a".into(),
            }),
            child_started("background"),
            ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::RootPhase {
                    phase: TurnPhase::Ready,
                },
            )),
        ],
    );
    let now = Instant::now();
    assert!(reap(&mut pool, now));
    assert!(reap(&mut pool, now + SERVED_IDLE / 2));
    assert!(releases(&pool).is_empty());
    emit(&mut pool, &run_id, vec![child_output("background")]);

    assert!(reap(&mut pool, now + SERVED_IDLE + SERVED_IDLE / 4));
    assert!(releases(&pool).is_empty());

    assert!(reap(&mut pool, now + SERVED_IDLE * 3));
    assert_eq!(releases(&pool), ["run-a"]);
}

#[test]
fn a_process_whose_kill_fails_is_still_dropped_and_does_not_block_other_reaps() {
    let mut pool = Pool::new(4);
    let first = pool.conversation("a");
    let second = pool.conversation("b");
    for conversation in [&first, &second] {
        let run_id = pool.send(conversation, "prompt");
        pool.finish(run_id);
    }
    pool.runner.0.lock().unwrap().fail_releases = true;
    let now = Instant::now();
    assert!(reap(&mut pool, now));

    assert!(!reap(&mut pool, now + SERVED_IDLE));
    assert_eq!(releases(&pool), ["run-a", "run-b"]);

    assert!(reap(&mut pool, now + SERVED_IDLE * 2));
    assert_eq!(releases(&pool), ["run-a", "run-b"]);
    pool.runner.0.lock().unwrap().fail_releases = false;
    pool.send(&first, "after");
    assert_eq!(
        pool.runner.0.lock().unwrap().starts,
        ["run-a", "run-b", "run-a"]
    );
}

#[test]
fn a_background_child_permission_after_the_turn_settled_is_asked_without_reopening_the_turn() {
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

    emit(
        &mut pool,
        &run_id,
        vec![
            child_started("background"),
            ClaudeRunnerEffect::PermissionRequest(ClaudePermissionRequest {
                request_id: "late".into(),
                tool_use_id: "tool-late".into(),
                tool_name: "write_file".into(),
                operation_preview: Some(serde_json::json!({"file_path": "/workspace-a/late.rs"})),
                child_id: Some("background".into()),
            }),
        ],
    );

    assert!(
        pool.runner
            .0
            .lock()
            .unwrap()
            .permission_responses
            .is_empty()
    );
    let pending = pool
        .ledger
        .pending_permission(&conversation, &AgentChatRunId(run_id.clone()))
        .unwrap()
        .unwrap();
    assert_eq!(pending.binding.decision_id.0, "late");
    assert_eq!(pending.request.child_id.as_deref(), Some("background"));
    let activity = pool
        .ledger
        .read_conversation_activity_page(&conversation.0, &run_id, 0, 64)
        .unwrap()
        .facts;
    assert!(activity.iter().any(|fact| matches!(
        fact,
        ConversationActivityFact::DecisionPending { decision_id, request: Some(_), .. }
            if decision_id == "late"
    )));
    assert!(!activity.iter().any(|fact| matches!(
        fact,
        ConversationActivityFact::DecisionSettled { decision_id, .. } if decision_id == "late"
    )));
    assert!(
        pool.ledger
            .list_run_turns(&run_id)
            .unwrap()
            .iter()
            .all(|turn| turn.phase == gent_types::DurableTurnPhase::Completed)
    );
}

#[test]
fn a_continuation_turn_claude_starts_after_its_result_is_its_own_turn_and_can_ask() {
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
    emit(
        &mut pool,
        &run_id,
        vec![
            ClaudeRunnerEffect::Fact(PublicWireFact::SessionStarted {
                provider_session_id: "session-run-a".into(),
            }),
            ClaudeRunnerEffect::ContinuationStarted,
            ClaudeRunnerEffect::PermissionRequest(ClaudePermissionRequest {
                request_id: "continuation".into(),
                tool_use_id: "tool-continuation".into(),
                tool_name: "write_file".into(),
                operation_preview: Some(serde_json::json!({"file_path": "/workspace-a/c.rs"})),
                child_id: None,
            }),
        ],
    );
    let turns = pool.ledger.list_run_turns(&run_id).unwrap();
    assert_eq!(turns.len(), 2);
    let continuation = turns.last().unwrap();
    assert_ne!(continuation.phase, gent_types::DurableTurnPhase::Completed);
    assert!(
        pool.runner
            .0
            .lock()
            .unwrap()
            .permission_responses
            .is_empty()
    );
    let pending = pool
        .ledger
        .pending_permission(&conversation, &AgentChatRunId(run_id.clone()))
        .unwrap()
        .unwrap();
    assert_eq!(pending.binding.turn_id, continuation.turn_id);
    emit(
        &mut pool,
        &run_id,
        vec![ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::RootPhase {
                phase: TurnPhase::Ready,
            },
        ))],
    );
    let turns = pool.ledger.list_run_turns(&run_id).unwrap();
    assert_eq!(turns.len(), 2);
    assert!(
        turns
            .iter()
            .all(|turn| turn.phase == gent_types::DurableTurnPhase::Completed)
    );
}

#[test]
fn a_child_without_a_terminal_report_does_not_hold_the_run_once_claude_reports_idle() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    let run_id = pool.send(&conversation, "a1");
    emit(
        &mut pool,
        &run_id,
        vec![
            ClaudeRunnerEffect::Fact(PublicWireFact::SessionStarted {
                provider_session_id: "session-run-a".into(),
            }),
            ClaudeRunnerEffect::SessionState { idle: false },
            child_started("never-reports"),
            ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::RootPhase {
                    phase: TurnPhase::Ready,
                },
            )),
        ],
    );
    assert!(pool.host.needs_drive());
    emit(
        &mut pool,
        &run_id,
        vec![ClaudeRunnerEffect::SessionState { idle: true }],
    );
    assert!(!pool.host.needs_drive());
}

#[test]
fn stop_with_no_open_turn_reaches_a_claude_session_still_running_background_work() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    let run_id = pool.send(&conversation, "a1");
    emit(
        &mut pool,
        &run_id,
        vec![
            ClaudeRunnerEffect::Fact(PublicWireFact::SessionStarted {
                provider_session_id: "session-run-a".into(),
            }),
            ClaudeRunnerEffect::SessionState { idle: false },
            child_started("background"),
            ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::RootPhase {
                    phase: TurnPhase::Ready,
                },
            )),
        ],
    );
    assert!(
        pool.ledger
            .list_run_turns(&run_id)
            .unwrap()
            .iter()
            .all(|turn| turn.phase.is_terminal())
    );
    assert!(pool.host.needs_drive());
    pool.host.interrupt(&run_id).unwrap();
    assert_eq!(
        pool.runner.0.lock().unwrap().interrupts,
        std::slice::from_ref(&run_id)
    );
    emit(
        &mut pool,
        &run_id,
        vec![ClaudeRunnerEffect::SessionState { idle: true }],
    );
    assert!(!pool.host.needs_drive());
    let now = Instant::now();
    assert!(reap(&mut pool, now));
    assert!(reap(&mut pool, now + SERVED_IDLE));
    assert_eq!(releases(&pool), [run_id]);
}

#[path = "claude_child_permission_tests.rs"]
mod child_permission;
