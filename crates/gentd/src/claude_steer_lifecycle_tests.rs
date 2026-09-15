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

fn steered_scenario() -> Scenario {
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
    ledger
        .steer_queued_agent_chat_prompt(
            &ReceiptId("steer".into()),
            HostEpoch(1),
            &AgentChatConversationId("conversation".into()),
            &queued.message.message_id,
        )
        .unwrap();
    runner
        .0
        .lock()
        .unwrap()
        .effects
        .push_back(vec![ClaudeRunnerEffect::Fact(
            PublicWireFact::SessionStarted {
                provider_session_id: "session".into(),
            },
        )]);
    host.tick().unwrap();
    assert_eq!(
        runner.0.lock().unwrap().steers,
        [(queued.message.message_id.clone(), "message-queued".into())]
    );
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
    let mut scenario = steered_scenario();
    let queued_id = scenario.queued.message.message_id.clone();
    scenario.runner.0.lock().unwrap().effects.push_back(vec![
        output("before"),
        ClaudeRunnerEffect::SteerConsumed {
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
}

#[test]
fn a_take_up_after_the_result_in_the_same_batch_becomes_the_next_turn_without_a_resend() {
    let mut scenario = steered_scenario();
    let queued_id = scenario.queued.message.message_id.clone();
    scenario.runner.0.lock().unwrap().effects.push_back(vec![
        output("first"),
        phase(TurnPhase::Ready),
        ClaudeRunnerEffect::SteerConsumed {
            message_id: queued_id,
        },
        output("second"),
        phase(TurnPhase::Ready),
    ]);
    scenario.host.tick().unwrap();

    let queued_turn = scenario.queued.message.turn_id.clone();
    assert_eq!(
        scenario
            .ledger
            .find_turn(&scenario.active.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        DurableTurnPhase::Completed
    );
    assert_eq!(
        scenario
            .ledger
            .find_turn(&queued_turn)
            .unwrap()
            .unwrap()
            .phase,
        DurableTurnPhase::Completed
    );
    assert_eq!(
        kinds(&scenario),
        [
            ("promptQueued".into(), queued_turn.clone()),
            ("promptReleased".into(), queued_turn.clone())
        ]
    );
    assert_eq!(turn_of(&scenario, "second"), queued_turn);
    let state = scenario.runner.0.lock().unwrap();
    assert_eq!((state.steers.len(), state.starts), (1, 1));
}

#[test]
fn an_interrupted_turn_returns_its_untaken_steer_to_the_queue_for_one_normal_launch() {
    let mut scenario = steered_scenario();
    scenario.host.interrupt("run").unwrap();
    scenario.runner.0.lock().unwrap().effects.push_back(vec![
        phase(TurnPhase::Failed),
        ClaudeRunnerEffect::Exited { code: Some(0) },
    ]);
    scenario.runner.0.lock().unwrap().active = false;
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
    assert_eq!(state.starts + state.resumes, 2);
}
