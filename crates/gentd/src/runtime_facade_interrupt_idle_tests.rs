use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger, RunLifecycleFactLedger,
};
use gent_protocol::AgentChatIntentFrame;
use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, ConversationActivityFact, ConversationActivityScope,
    ConversationWorkStatus, DurableTurnPhase, HostEpoch, NormalizedProviderEvent, ReceiptId,
    RunLifecycleFact, ToolActivity, ToolPhase,
};

use super::interrupt_ended_tests::status_turn;
use super::{RuntimeFacade, facade, root_with_selection, running_on};
use crate::api::RuntimeApi;

const BACKGROUND_COMMAND: &str = "background-command";

fn claude_without_a_live_session(runtime: &RuntimeFacade, key: &str) -> AgentChatConversationId {
    root_with_selection(
        runtime,
        key,
        ".",
        AgentChatSelection {
            provider: AgentChatProvider::Claude,
            model: "haiku".into(),
            effort: AgentChatEffort::Medium,
            mode: AgentChatMode::Agent,
        },
    )
}

fn open_background_command(
    runtime: &RuntimeFacade,
    conversation: &AgentChatConversationId,
    run_id: &str,
    turn_id: &str,
) {
    let ledger = runtime.conversation_links().unwrap();
    let fact = ConversationActivityFact::ToolActivity {
        scope: ConversationActivityScope {
            conversation_id: conversation.0.clone(),
            run_id: run_id.into(),
            turn_id: turn_id.into(),
            host_epoch: HostEpoch(1),
            cursor: 0,
        },
        activity: ToolActivity {
            tool_use_id: BACKGROUND_COMMAND.into(),
            tool_name: "Bash".into(),
            phase: ToolPhase::Background,
            input: None,
            output_digest: None,
            parent_tool_use_id: None,
        },
    };
    let cursor = ledger
        .append_event(&gent_types::Event {
            cursor: 0,
            event_id: BACKGROUND_COMMAND.into(),
            receipt_id: ReceiptId("background-command-receipt".into()),
            host_epoch: HostEpoch(1),
            kind: "providerActivity".into(),
            payload: serde_json::json!({
                "conversationId": conversation.0,
                "runId": run_id,
                "turnId": turn_id,
                "activity": fact,
            }),
        })
        .unwrap()
        .cursor;
    ledger
        .append_conversation_activity(&gent_core::with_activity_cursor(fact, cursor))
        .unwrap();
}

fn record_turn_ended(runtime: &RuntimeFacade, run_id: &str, turn_id: &str) {
    let ledger = runtime.conversation_links().unwrap();
    let event = NormalizedProviderEvent::TurnEnded {
        turn_id: turn_id.into(),
    };
    let cursor = ledger
        .append_event(&gent_types::Event {
            cursor: 0,
            event_id: "turn-ended".into(),
            receipt_id: ReceiptId("turn-ended-receipt".into()),
            host_epoch: HostEpoch(1),
            kind: "providerLifecycle".into(),
            payload: serde_json::json!({ "runId": run_id, "event": event }),
        })
        .unwrap()
        .cursor;
    ledger
        .append_run_lifecycle_fact(&RunLifecycleFact {
            run_id: run_id.into(),
            event_id: "turn-ended".into(),
            host_epoch: HostEpoch(1),
            cursor,
            lifecycle: gent_types::NormalizedSessionLifecycle::Event { event },
        })
        .unwrap();
}

fn command_work(runtime: &RuntimeFacade, run_id: &str) -> ConversationWorkStatus {
    runtime
        .conversation_links()
        .unwrap()
        .run_live_status(run_id)
        .unwrap()
        .unwrap()
        .status
        .command_work
}

fn command_phases(
    runtime: &RuntimeFacade,
    conversation: &AgentChatConversationId,
    run_id: &str,
) -> Vec<ToolPhase> {
    runtime
        .conversation_links()
        .unwrap()
        .read_conversation_activity_page(&conversation.0, run_id, 0, 64)
        .unwrap()
        .facts
        .into_iter()
        .filter_map(|fact| match fact {
            ConversationActivityFact::ToolActivity { activity, .. }
                if activity.tool_use_id == BACKGROUND_COMMAND =>
            {
                Some(activity.phase)
            }
            _ => None,
        })
        .collect()
}

async fn stop_wire(
    runtime: &RuntimeFacade,
    conversation: &AgentChatConversationId,
    run_id: &str,
    key: &str,
) -> serde_json::Value {
    let error = runtime
        .agent_chat_intent(AgentChatIntentFrame::Interrupt {
            request_id: AgentChatRequestId(format!("stop-{key}")),
            receipt_id: ReceiptId(format!("stop-receipt-{key}")),
            conversation_id: conversation.clone(),
            run_id: AgentChatRunId(run_id.into()),
            expected_turn_id: None,
        })
        .expect_err("a stop the provider cannot deliver is rejected");
    let mut bytes = Vec::new();
    error.write(&mut bytes).await.unwrap();
    serde_json::from_slice(&bytes[4..]).unwrap()
}

#[tokio::test]
async fn stopping_a_conversation_with_nothing_running_ends_its_open_commands_and_says_so() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let conversation = claude_without_a_live_session(&runtime, "idle");
    let prompt = running_on(&runtime, &conversation, "idle", AgentChatProvider::Claude);
    let (run_id, turn_id) = status_turn(&runtime, &conversation);
    open_background_command(&runtime, &conversation, &run_id, &turn_id);
    runtime
        .conversation_links()
        .unwrap()
        .settle_agent_chat_prompt_terminal(
            &prompt,
            "daemon",
            HostEpoch(1),
            DurableTurnPhase::Completed,
        )
        .unwrap();
    record_turn_ended(&runtime, &run_id, &turn_id);
    assert_eq!(
        command_work(&runtime, &run_id),
        ConversationWorkStatus::Waiting
    );

    let answer = stop_wire(&runtime, &conversation, &run_id, "idle").await;

    assert_eq!(
        answer,
        serde_json::json!({"type": "error", "body": {
            "code": "nothingToInterrupt",
            "message": "nothing is running in this conversation to interrupt"
        }})
    );
    assert_eq!(
        command_phases(&runtime, &conversation, &run_id),
        [ToolPhase::Background, ToolPhase::Interrupted]
    );
    assert_eq!(
        command_work(&runtime, &run_id),
        ConversationWorkStatus::None
    );
}

#[tokio::test]
async fn a_stop_that_cannot_reach_a_running_turn_is_not_reported_as_nothing_to_interrupt() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let conversation = claude_without_a_live_session(&runtime, "busy");
    running_on(&runtime, &conversation, "busy", AgentChatProvider::Claude);
    let (run_id, turn_id) = status_turn(&runtime, &conversation);
    open_background_command(&runtime, &conversation, &run_id, &turn_id);

    let answer = stop_wire(&runtime, &conversation, &run_id, "busy").await;

    assert_eq!(answer["body"]["code"], "agentChatRejected");
    assert_eq!(
        command_phases(&runtime, &conversation, &run_id),
        [ToolPhase::Background]
    );
}
