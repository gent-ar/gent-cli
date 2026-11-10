use std::time::{Duration, Instant};

use gent_ports::{AgentChatReadLedger, AgentChatSelectionLedger, ConversationLedger, Ledger};
use gent_protocol::ProviderWarmOutcome;
use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRunId,
    AgentChatSelection, AgentChatSelectionSwitch, ContextPolicy, HostEpoch, ReceiptId,
};

use super::claude_warm_pool_tests::Pool;
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
    pool.host
        .tend_warm_pool(&WarmPoolRequest {
            now,
            target: Some(WarmTarget {
                conversation_id: conversation.0.clone(),
                run_id,
            }),
        })
        .unwrap()
        .unwrap()
}

fn reap(pool: &mut Pool, now: Instant) {
    pool.host
        .tend_warm_pool(&WarmPoolRequest { now, target: None })
        .unwrap();
}

fn switch(pool: &Pool, conversation: &AgentChatConversationId, provider: AgentChatProvider) {
    pool.ledger
        .switch_agent_chat_selection(&AgentChatSelectionSwitch {
            receipt_id: ReceiptId("switch".into()),
            idempotency_key: "switch".into(),
            host_epoch: HostEpoch(1),
            conversation_id: conversation.clone(),
            parent_run_id: AgentChatRunId("run-a".into()),
            run_id: AgentChatRunId("run-a-child".into()),
            selection: AgentChatSelection {
                provider,
                model: "other-model".into(),
                effort: AgentChatEffort::Low,
                mode: AgentChatMode::Agent,
            },
            context_policy: ContextPolicy::Preserve,
        })
        .unwrap();
}

fn observed(pool: &Pool) -> (Vec<String>, Vec<String>, Vec<String>) {
    let state = pool.runner.0.lock().unwrap();
    (
        state.starts.clone(),
        state.releases.clone(),
        state.submitted.clone(),
    )
}

#[test]
fn a_prompt_after_a_claude_warm_up_adopts_the_spawned_process() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    let now = Instant::now();

    assert_eq!(
        warm(&mut pool, &conversation, now),
        ProviderWarmOutcome::Warming
    );
    assert_eq!(
        warm(&mut pool, &conversation, now),
        ProviderWarmOutcome::AlreadyWarm
    );
    let run_id = pool.send(&conversation, "hello");
    pool.finish(run_id);

    let (starts, releases, submitted) = observed(&pool);
    assert_eq!(starts, ["run-a"]);
    assert!(releases.is_empty());
    assert_eq!(submitted.len(), 1);
    assert!(submitted[0].ends_with("hello"));
}

#[test]
fn a_claude_warm_up_during_a_turn_is_a_no_op() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    pool.send(&conversation, "working");

    assert_eq!(
        warm(&mut pool, &conversation, Instant::now()),
        ProviderWarmOutcome::TurnActive
    );
    assert_eq!(observed(&pool).0, ["run-a"]);
}

#[test]
fn a_switch_from_claude_to_codex_reaps_the_warm_claude_process() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    let now = Instant::now();
    warm(&mut pool, &conversation, now);
    switch(&pool, &conversation, AgentChatProvider::Codex);

    reap(&mut pool, now);

    assert_eq!(observed(&pool).1, ["run-a"]);
    assert_eq!(pool.host.active_len(), 0);
}

#[test]
fn a_claude_model_switch_warms_a_new_process_for_the_new_run_only() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    let now = Instant::now();
    warm(&mut pool, &conversation, now);
    switch(&pool, &conversation, AgentChatProvider::Claude);

    assert_eq!(
        warm(&mut pool, &conversation, now),
        ProviderWarmOutcome::Warming
    );
    let run_id = pool.send(&conversation, "on the new model");
    pool.finish(run_id);

    let (starts, releases, _) = observed(&pool);
    assert_eq!(starts, ["run-a", "run-a-child"]);
    assert_eq!(releases, ["run-a"]);
}

#[test]
fn an_unprompted_claude_process_is_reaped_without_a_ledger_trace() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    let now = Instant::now();
    warm(&mut pool, &conversation, now);

    reap(&mut pool, now + Duration::from_secs(60));
    assert!(observed(&pool).1.is_empty());
    reap(&mut pool, now + Duration::from_secs(120));

    assert_eq!(observed(&pool).1, ["run-a"]);
    assert!(
        pool.ledger
            .find_run_session_binding("run-a")
            .unwrap()
            .is_none()
    );
    assert!(pool.ledger.list_run_turns("run-a").unwrap().is_empty());
}

#[test]
fn a_failed_claude_warm_up_leaves_the_prompt_to_start_normally() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    pool.runner.0.lock().unwrap().fail_starts = 1;

    assert_eq!(
        warm(&mut pool, &conversation, Instant::now()),
        ProviderWarmOutcome::Unavailable
    );
    let run_id = pool.send(&conversation, "hello");
    pool.finish(run_id);

    let (starts, _, submitted) = observed(&pool);
    assert_eq!(starts, ["run-a"]);
    assert!(submitted.is_empty(), "the prompt launched its own process");
}

#[test]
fn a_failed_claude_warm_up_is_not_relaunched_by_repeated_requests_until_its_backoff() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    pool.runner.0.lock().unwrap().fail_starts = 1;
    let failed_at = Instant::now();

    assert_eq!(
        warm(&mut pool, &conversation, failed_at),
        ProviderWarmOutcome::Unavailable
    );
    for seconds in [0, 1, 30, 59] {
        assert_eq!(
            warm(
                &mut pool,
                &conversation,
                failed_at + Duration::from_secs(seconds)
            ),
            ProviderWarmOutcome::Unavailable
        );
    }
    assert!(observed(&pool).0.is_empty());
    assert_eq!(
        warm(
            &mut pool,
            &conversation,
            failed_at + Duration::from_secs(61)
        ),
        ProviderWarmOutcome::Warming
    );
    assert_eq!(observed(&pool).0, ["run-a"]);
}

#[test]
fn a_full_claude_pool_of_busy_runs_refuses_a_warm_up() {
    let mut pool = Pool::new(1);
    let busy = pool.conversation("a");
    let other = pool.conversation("b");
    pool.send(&busy, "working");

    assert_eq!(
        warm(&mut pool, &other, Instant::now()),
        ProviderWarmOutcome::PoolFull
    );
}
