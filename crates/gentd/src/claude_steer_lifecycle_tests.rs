use gent_drivers::claude_runner::ClaudeRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{
    AgentChatPromptLedger, AgentChatQueuedPromptLedger, AgentChatReadLedger,
    AgentChatWorkspaceLedger, ConversationActivityLedger, ConversationLedger,
};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatProvider,
    AgentChatRequestId, AgentChatRunId, AgentChatSelection, CapabilitySet, DurableTurnPhase,
    HostEpoch, NormalizedLifecycleSignal, NormalizedProviderEvent, ReceiptId, TurnPhase,
    WorkspaceRecord,
};

use super::{Resolver, Runner, compatibility, profile};
use crate::approved_claude_host::ApprovedClaudeHost;
use crate::public_driver_runtime::PublicDriversRuntime;
#[path = "claude_queued_cancel_tests.rs"]
mod queued_cancel;
#[path = "claude_stop_withdraw_tests.rs"]
mod stop_withdraw;
#[path = "claude_turn_tool_settlement_tests.rs"]
mod tool_settlement;
#[path = "claude_steer_write_failure_tests.rs"]
mod write_failure;

struct Scenario {
    ledger: SqliteLedger,
    runner: Runner,
    host: ApprovedClaudeHost<SqliteLedger, Runner, Resolver>,
    active: AgentChatPromptSaved,
    queued: AgentChatPromptSaved,
}

fn save(
    ledger: &SqliteLedger,
    key: &str,
    disposition: AgentChatPromptDisposition,
) -> AgentChatPromptSaved {
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(format!("request-{key}")),
            receipt_id: ReceiptId(format!("receipt-{key}")),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation".into()),
            disposition,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: format!("message-{key}"),
        })
        .unwrap();
    crate::readiness_test_support::release(ledger, &saved);
    saved
}

fn joined_scenario() -> Scenario {
    let mut scenario = queued_before_session();
    scenario
        .runner
        .0
        .lock()
        .unwrap()
        .effects
        .push_back(vec![ClaudeRunnerEffect::Fact(
            PublicWireFact::SessionStarted {
                provider_session_id: "session".into(),
            },
        )]);
    scenario.host.tick().unwrap();
    assert_eq!(
        scenario.runner.0.lock().unwrap().steers,
        [(
            scenario.queued.message.message_id.clone(),
            "message-queued".into()
        )]
    );
    scenario
}

fn queued_before_session() -> Scenario {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation".into()),
                run_id: AgentChatRunId("run".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "claude-test".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    let active = save(&ledger, "active", AgentChatPromptDisposition::Send);
    let queued = save(&ledger, "queued", AgentChatPromptDisposition::Queue);
    let runner = Runner::default();
    let compatibility = compatibility();
    let runtime = PublicDriversRuntime::new(
        profile(&compatibility),
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger.clone(),
        compatibility,
        runner.clone(),
        Resolver,
    )
    .unwrap();
    let mut host = ApprovedClaudeHost::new(runtime, "daemon".into(), HostEpoch(1), 1, None);
    host.tick().unwrap();
    Scenario {
        ledger,
        runner,
        host,
        active,
        queued,
    }
}

fn output(text: &str) -> ClaudeRunnerEffect {
    ClaudeRunnerEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::Output {
        text: text.into(),
        is_partial: false,
    }))
}

fn phase(phase: TurnPhase) -> ClaudeRunnerEffect {
    ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::RootPhase { phase },
    ))
}

fn kinds(scenario: &Scenario) -> Vec<(String, String)> {
    scenario
        .ledger
        .read_conversation_activity_page("conversation", "run", 0, 50)
        .unwrap()
        .facts
        .iter()
        .map(|fact| {
            let value = serde_json::to_value(fact).unwrap();
            (
                value["type"].as_str().unwrap().to_owned(),
                value["turnId"].as_str().unwrap().to_owned(),
            )
        })
        .filter(|(kind, _)| kind.starts_with("prompt"))
        .collect()
}

fn turn_of(scenario: &Scenario, text: &str) -> String {
    scenario
        .ledger
        .read_agent_chat_transcript("conversation", None, 100)
        .unwrap()
        .events
        .into_iter()
        .find(|event| event.text == text)
        .unwrap()
        .turn_id
}

#[test]
fn a_take_up_between_tool_output_and_the_result_is_recorded_in_the_running_turn() {
    let mut scenario = joined_scenario();
    assert!(scenario.host.needs_drive());
    let queued_id = scenario.queued.message.message_id.clone();
    scenario.runner.0.lock().unwrap().effects.push_back(vec![
        output("before"),
        ClaudeRunnerEffect::MessageStarted {
            message_id: queued_id.clone(),
        },
        output("after"),
        phase(TurnPhase::Ready),
    ]);
    scenario.host.tick().unwrap();

    let active_turn = scenario.active.message.turn_id.clone();
    assert_eq!(
        scenario
            .ledger
            .find_turn(&active_turn)
            .unwrap()
            .unwrap()
            .phase,
        DurableTurnPhase::Completed
    );
    assert_eq!(
        scenario
            .ledger
            .find_turn(&scenario.queued.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        DurableTurnPhase::Completed
    );
    assert_eq!(
        kinds(&scenario),
        [
            (
                "promptQueued".into(),
                scenario.queued.message.turn_id.clone()
            ),
            ("promptSteered".into(), active_turn.clone())
        ]
    );
    assert_eq!(turn_of(&scenario, "after"), active_turn);
    assert_eq!(scenario.runner.0.lock().unwrap().steers.len(), 1);
    assert!(!scenario.host.needs_drive());
}

#[path = "claude_steer_legacy_late_tests.rs"]
mod legacy_late;

#[test]
fn a_prompt_queued_before_the_claude_session_starts_waits_and_never_fails_the_turn() {
    {
        let mut scenario = queued_before_session();
        scenario.host.tick().unwrap();
        scenario.host.tick().unwrap();
        assert!(scenario.runner.0.lock().unwrap().steers.is_empty());
        let active_turn = scenario.active.message.turn_id.clone();
        assert_eq!(
            scenario
                .ledger
                .find_turn(&active_turn)
                .unwrap()
                .unwrap()
                .phase,
            DurableTurnPhase::Active
        );
        scenario
            .runner
            .0
            .lock()
            .unwrap()
            .effects
            .push_back(vec![ClaudeRunnerEffect::Fact(
                PublicWireFact::SessionStarted {
                    provider_session_id: "session".into(),
                },
            )]);
        scenario.host.tick().unwrap();
        assert_eq!(
            scenario.runner.0.lock().unwrap().steers,
            [(
                scenario.queued.message.message_id.clone(),
                "message-queued".into()
            )]
        );
        scenario.runner.0.lock().unwrap().effects.push_back(vec![
            ClaudeRunnerEffect::MessageStarted {
                message_id: scenario.queued.message.message_id.clone(),
            },
            output("after"),
            phase(TurnPhase::Ready),
        ]);
        scenario.host.tick().unwrap();
        for turn in [&active_turn, &scenario.queued.message.turn_id] {
            assert_eq!(
                scenario.ledger.find_turn(turn).unwrap().unwrap().phase,
                DurableTurnPhase::Completed
            );
        }
        assert_eq!(turn_of(&scenario, "after"), active_turn);
    }
}

#[test]
fn an_interrupted_turn_returns_its_untaken_prompt_to_the_queue_for_one_normal_launch() {
    let mut scenario = joined_scenario();
    assert!(scenario.host.needs_drive());
    scenario.host.interrupt("run").unwrap();
    scenario.runner.0.lock().unwrap().effects.push_back(vec![
        ClaudeRunnerEffect::MessageCancelled {
            message_id: scenario.queued.message.message_id.clone(),
        },
        phase(TurnPhase::Interrupted),
    ]);
    scenario.host.tick().unwrap();
    scenario.host.tick().unwrap();

    assert_eq!(
        scenario
            .ledger
            .find_turn(&scenario.active.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        DurableTurnPhase::Interrupted
    );
    let queued_turn = scenario.queued.message.turn_id.clone();
    assert_eq!(
        kinds(&scenario),
        [
            ("promptQueued".into(), queued_turn.clone()),
            ("promptReleased".into(), queued_turn)
        ]
    );
    let state = scenario.runner.0.lock().unwrap();
    assert_eq!(
        (state.starts + state.resumes, state.protocol_interrupts),
        (1, 1)
    );
    assert!(state.signals.is_empty());
}
