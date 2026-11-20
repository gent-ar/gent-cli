use gent_drivers::codex_control::CodexControlRequest;
use gent_drivers::codex_runner::CodexRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{ConversationLedger, PendingPermissionLedger};
use gent_types::{
    AgentChatRunId, DurableTurnPhase, NormalizedLifecycleSignal, NormalizedProviderEvent, TurnPhase,
};

use super::codex_warm_pool_tests::Pool;

fn emit(pool: &mut Pool, run_id: &str, effects: Vec<CodexRunnerEffect>) {
    pool.runner
        .state
        .lock()
        .unwrap()
        .run_effects
        .entry(run_id.into())
        .or_default()
        .push_back(effects);
    pool.host.tick().unwrap();
}

fn provider_turn_started() -> Vec<CodexRunnerEffect> {
    vec![
        CodexRunnerEffect::ProviderTurnStarted,
        CodexRunnerEffect::Fact(PublicWireFact::Event(
            NormalizedProviderEvent::TurnStarted {
                turn_id: "native-turn-2".into(),
            },
        )),
    ]
}

fn phases(pool: &Pool, run_id: &str) -> Vec<DurableTurnPhase> {
    pool.ledger
        .list_run_turns(run_id)
        .unwrap()
        .into_iter()
        .map(|turn| turn.phase)
        .collect()
}

#[test]
fn a_turn_codex_starts_on_its_own_is_a_gent_turn_that_owns_its_approval_and_completion() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    let run_id = pool.send(&conversation, "a1");
    pool.finish(&run_id);

    emit(&mut pool, &run_id, provider_turn_started());
    let turns = pool.ledger.list_run_turns(&run_id).unwrap();
    assert_eq!(turns.len(), 2);
    let provider_turn = turns.last().unwrap().turn_id.clone();
    assert_ne!(turns.last().unwrap().phase, DurableTurnPhase::Completed);
    assert!(pool.host.needs_drive());

    emit(
        &mut pool,
        &run_id,
        vec![CodexRunnerEffect::ControlRequest(CodexControlRequest {
            request_id: serde_json::json!(7),
            request_key: "7".into(),
            method: "item/commandExecution/requestApproval".into(),
            tool_use_id: "call-7".into(),
            tool_name: "Bash".into(),
            input: None,
            child_id: None,
        })],
    );
    let pending = pool
        .ledger
        .pending_permission(&conversation, &AgentChatRunId(run_id.clone()))
        .unwrap()
        .unwrap();
    assert_eq!(pending.binding.turn_id, provider_turn);

    emit(
        &mut pool,
        &run_id,
        vec![CodexRunnerEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::RootPhase {
                phase: TurnPhase::Ready,
            },
        ))],
    );
    assert_eq!(
        phases(&pool, &run_id),
        [DurableTurnPhase::Completed, DurableTurnPhase::Completed]
    );
    assert!(!pool.host.needs_drive());
}

#[test]
fn codex_reporting_its_thread_idle_closes_a_provider_turn_it_never_completed() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    let run_id = pool.send(&conversation, "a1");
    pool.finish(&run_id);
    emit(&mut pool, &run_id, provider_turn_started());
    emit(
        &mut pool,
        &run_id,
        vec![CodexRunnerEffect::SessionState { idle: false }],
    );
    assert!(pool.host.needs_drive());

    emit(
        &mut pool,
        &run_id,
        vec![CodexRunnerEffect::SessionState { idle: true }],
    );

    assert_eq!(
        phases(&pool, &run_id),
        [DurableTurnPhase::Completed, DurableTurnPhase::Completed]
    );
    assert!(!pool.host.needs_drive());
}

#[test]
fn a_provider_turn_does_not_open_while_the_client_turn_is_live() {
    let mut pool = Pool::new(4);
    let conversation = pool.conversation("a");
    let run_id = pool.send(&conversation, "a1");
    emit(&mut pool, &run_id, provider_turn_started());
    assert_eq!(pool.ledger.list_run_turns(&run_id).unwrap().len(), 1);
}
