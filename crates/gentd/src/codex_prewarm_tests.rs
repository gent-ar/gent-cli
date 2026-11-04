use std::time::{Duration, Instant};

use gent_drivers::codex_runner::CodexRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{
    AgentChatReadLedger, AgentChatSelectionLedger, ConversationLedger, Ledger, PolicyLedger,
};
use gent_protocol::ProviderWarmOutcome;
use gent_types::{
    AgentChatConversationId, AgentChatProvider, AgentChatRunId, AgentChatSelection,
    AgentChatSelectionSwitch, ContextPolicy, HostEpoch, PermissionMode, PolicyRecord, PolicyScope,
    ReceiptId,
};

use super::codex_warm_pool_tests::Pool;
use crate::codex_prompt_lifecycle_tests::selection;
use crate::provider_warm_pool::{WarmPoolRequest, WarmTarget};

fn warm(
    pool: &mut Pool,
    conversation: &AgentChatConversationId,
    now: Instant,
) -> ProviderWarmOutcome {
    let run_id = pool
        .ledger
        .read_agent_chat_detail(&conversation.0)
        .unwrap()
        .current_run_id;
    warm_run(pool, conversation, &run_id, now)
}

fn warm_run(
    pool: &mut Pool,
    conversation: &AgentChatConversationId,
    run_id: &str,
    now: Instant,
) -> ProviderWarmOutcome {
    pool.host
        .tend_warm_pool(&WarmPoolRequest {
            now,
            target: Some(WarmTarget {
                conversation_id: conversation.0.clone(),
                run_id: run_id.into(),
            }),
        })
        .unwrap()
        .unwrap()
}

fn reap(pool: &mut Pool, now: Instant) {
    assert_eq!(
        pool.host
            .tend_warm_pool(&WarmPoolRequest { now, target: None })
            .unwrap(),
        None
    );
}

fn handshake(pool: &Pool, run_id: &str) {
    pool.runner
        .state
        .lock()
        .unwrap()
        .run_effects
        .entry(run_id.into())
        .or_default()
        .push_back(vec![CodexRunnerEffect::Fact(
            PublicWireFact::SessionStarted {
                provider_session_id: format!("thread-{run_id}"),
            },
        )]);
}

fn switch(
    pool: &Pool,
    conversation: &AgentChatConversationId,
    parent: &str,
    to: AgentChatSelection,
) {
    pool.ledger
        .switch_agent_chat_selection(&AgentChatSelectionSwitch {
            receipt_id: ReceiptId(format!("switch-{parent}")),
            idempotency_key: format!("switch-{parent}"),
            host_epoch: HostEpoch(1),
            conversation_id: conversation.clone(),
            parent_run_id: AgentChatRunId(parent.into()),
            run_id: AgentChatRunId(format!("{parent}-child")),
            selection: to,
            context_policy: ContextPolicy::Preserve,
        })
        .unwrap();
}

fn starts(pool: &Pool) -> usize {
    pool.runner.state.lock().unwrap().starts
}

fn releases(pool: &Pool) -> Vec<String> {
    pool.runner.state.lock().unwrap().releases.clone()
}

#[test]
fn a_prompt_after_a_warm_up_adopts_the_warm_process() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    let now = Instant::now();

    assert_eq!(
        warm(&mut pool, &conversation, now),
        ProviderWarmOutcome::Warming
    );
    handshake(&pool, "run-a");
    pool.host.tick().unwrap();
    pool.send(&conversation, "hello");

    let (starts, releases, submitted) = pool.counts();
    assert_eq!(starts, 1, "the prompt must not respawn the warm process");
    assert!(releases.is_empty());
    assert_eq!(submitted.len(), 1);
    assert!(submitted[0].ends_with("hello"));
    assert!(
        pool.ledger
            .find_run_session_binding("run-a")
            .unwrap()
            .is_some()
    );
}

#[test]
fn repeated_warm_ups_keep_one_process() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    let now = Instant::now();

    assert_eq!(
        warm(&mut pool, &conversation, now),
        ProviderWarmOutcome::Warming
    );
    for _ in 0..3 {
        assert_eq!(
            warm(&mut pool, &conversation, now),
            ProviderWarmOutcome::AlreadyWarm
        );
    }
    assert_eq!(starts(&pool), 1);
}

#[test]
fn a_warm_up_during_an_active_turn_changes_nothing() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    pool.send(&conversation, "working");

    assert_eq!(
        warm(&mut pool, &conversation, Instant::now()),
        ProviderWarmOutcome::TurnActive
    );
    assert_eq!(starts(&pool), 1);
    assert!(releases(&pool).is_empty());
}

#[test]
fn a_model_switch_stops_the_warm_process_and_warms_only_the_new_run() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    let now = Instant::now();
    warm(&mut pool, &conversation, now);
    let mut model = selection();
    model.model = "gpt-5.6-terra".into();
    switch(&pool, &conversation, "run-a", model);

    assert_eq!(
        warm_run(&mut pool, &conversation, "run-a", now),
        ProviderWarmOutcome::Superseded
    );
    assert_eq!(releases(&pool), ["run-a"]);
    assert_eq!(
        warm(&mut pool, &conversation, now),
        ProviderWarmOutcome::Warming
    );
    pool.send(&conversation, "on the new model");
    assert_eq!(starts(&pool), 2);
}

#[test]
fn a_switch_to_claude_reaps_the_warm_codex_process() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    let now = Instant::now();
    warm(&mut pool, &conversation, now);
    switch(
        &pool,
        &conversation,
        "run-a",
        AgentChatSelection {
            provider: AgentChatProvider::Claude,
            model: "haiku".into(),
            ..selection()
        },
    );

    reap(&mut pool, now);

    assert_eq!(releases(&pool), ["run-a"]);
    assert_eq!(pool.host.active_len(), 0);
}

#[test]
fn a_switch_immediately_after_a_warm_up_never_reuses_the_unready_process() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    let now = Instant::now();
    warm(&mut pool, &conversation, now);
    let mut model = selection();
    model.model = "gpt-5.6-terra".into();
    switch(&pool, &conversation, "run-a", model);

    let run_id = pool.send(&conversation, "after the switch");

    assert_eq!(run_id, "run-a-child");
    assert_eq!(releases(&pool), ["run-a"]);
    assert_eq!(starts(&pool), 2);
    assert!(
        pool.counts().2.is_empty(),
        "nothing was submitted to the stale process"
    );
}

#[test]
fn an_unprompted_warm_process_is_reaped_without_a_ledger_trace() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    let now = Instant::now();
    warm(&mut pool, &conversation, now);
    handshake(&pool, "run-a");
    pool.host.tick().unwrap();

    reap(&mut pool, now + Duration::from_secs(119));
    assert!(releases(&pool).is_empty());
    reap(&mut pool, now + Duration::from_secs(121));

    assert_eq!(releases(&pool), ["run-a"]);
    assert!(
        pool.ledger
            .find_run_session_binding("run-a")
            .unwrap()
            .is_none()
    );
    assert!(pool.ledger.list_run_turns("run-a").unwrap().is_empty());
    let transcript = pool
        .ledger
        .read_agent_chat_transcript(&conversation.0, None, 100)
        .unwrap();
    assert!(transcript.events.is_empty());
}

#[test]
fn a_served_process_is_reaped_only_after_the_longer_idle_timeout() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    let run_id = pool.send(&conversation, "first");
    pool.finish(&run_id);
    let now = Instant::now();

    reap(&mut pool, now);
    reap(&mut pool, now + Duration::from_secs(9 * 60));
    assert!(releases(&pool).is_empty());
    reap(&mut pool, now + Duration::from_secs(10 * 60));

    assert_eq!(releases(&pool), ["run-a"]);
}

#[test]
fn a_failed_warm_up_is_reported_and_the_prompt_still_starts() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    pool.runner.state.lock().unwrap().fail_starts = 1;

    assert_eq!(
        warm(&mut pool, &conversation, Instant::now()),
        ProviderWarmOutcome::Unavailable
    );
    pool.send(&conversation, "hello");

    assert_eq!(starts(&pool), 1);
    assert!(
        pool.counts().2.is_empty(),
        "the prompt started its own process"
    );
}

#[test]
fn a_full_pool_of_busy_runs_refuses_a_warm_up() {
    let mut pool = Pool::new(1);
    let busy = pool.conversation("a");
    let idle = pool.conversation("b");
    pool.send(&busy, "working");

    assert_eq!(
        warm(&mut pool, &idle, Instant::now()),
        ProviderWarmOutcome::PoolFull
    );
    assert_eq!(starts(&pool), 1);
}

#[test]
fn a_permission_policy_change_never_reuses_the_warm_process() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    warm(&mut pool, &conversation, Instant::now());
    pool.ledger
        .save_policy(&PolicyRecord {
            policy_id: "workspace-a-revision-2".into(),
            workspace_id: "workspace-a".into(),
            scope: PolicyScope::ProviderPermissions,
            revision: 2,
            mode: PermissionMode::AskEveryTime,
            containment: gent_types::Containment::Sandboxed,
            allowed_tools: vec!["Read".into()],
            allowed_categories: Vec::new(),
        })
        .unwrap();

    pool.send(&conversation, "with the new policy");

    assert_eq!(releases(&pool), ["run-a"]);
    assert_eq!(starts(&pool), 2);
    assert!(
        pool.counts().2.is_empty(),
        "the stale process received no prompt"
    );
}

#[test]
fn a_changed_mcp_configuration_never_reuses_the_warm_process() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    warm(&mut pool, &conversation, Instant::now());
    pool.runner.state.lock().unwrap().mcp_servers = Some(serde_json::json!({}));

    pool.send(&conversation, "with new tools");

    assert_eq!(releases(&pool), ["run-a"]);
    assert_eq!(starts(&pool), 2);
}
