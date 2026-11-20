use gent_ports::{AgentChatPromptDispatchLedger, Ledger, RunLifecycleFactLedger};
use gent_protocol::AgentChatIntentFrame;
use gent_types::{
    AgentChatConversationId, AgentChatRequestId, AgentChatRunId, DurableTurnPhase, HostEpoch,
    NormalizedProviderEvent, ReceiptId, RunLifecycleFact,
};

use super::{RuntimeFacade, facade, root, running};
use crate::api::RuntimeApi;

fn status_turn(
    runtime: &RuntimeFacade,
    conversation: &AgentChatConversationId,
) -> (String, String) {
    let status = runtime.conversation_status(&conversation.0).unwrap();
    let run = status
        .runs
        .into_iter()
        .find(|run| run.active_turn_id.is_some())
        .expect("the status read names the active turn");
    (run.run_id, run.active_turn_id.unwrap())
}

fn open_child(runtime: &RuntimeFacade, run_id: &str) {
    let ledger = runtime.conversation_links().unwrap();
    let event = NormalizedProviderEvent::ChildStarted {
        child_id: "background".into(),
        parent_tool_use_id: "tool".into(),
    };
    let cursor = ledger
        .append_event(&gent_types::Event {
            cursor: 0,
            event_id: "child-started".into(),
            receipt_id: ReceiptId("child-receipt".into()),
            host_epoch: HostEpoch(1),
            kind: "providerLifecycle".into(),
            payload: serde_json::json!({ "runId": run_id, "event": event }),
        })
        .unwrap()
        .cursor;
    ledger
        .append_run_lifecycle_fact(&RunLifecycleFact {
            run_id: run_id.into(),
            event_id: "child-started".into(),
            host_epoch: HostEpoch(1),
            cursor,
            lifecycle: gent_types::NormalizedSessionLifecycle::Event { event },
        })
        .unwrap();
}

async fn interrupt_wire(
    runtime: &RuntimeFacade,
    conversation: &AgentChatConversationId,
    run_id: &str,
    turn_id: &str,
) -> serde_json::Value {
    let error = runtime
        .agent_chat_intent(AgentChatIntentFrame::Interrupt {
            request_id: AgentChatRequestId(format!("interrupt-{turn_id}")),
            receipt_id: ReceiptId(format!("interrupt-receipt-{turn_id}")),
            conversation_id: conversation.clone(),
            run_id: AgentChatRunId(run_id.into()),
            expected_turn_id: Some(turn_id.into()),
        })
        .expect_err("an interrupt of an ended turn is rejected");
    let mut bytes = Vec::new();
    error.write(&mut bytes).await.unwrap();
    serde_json::from_slice(&bytes[4..]).unwrap()
}

#[tokio::test]
async fn interrupting_a_turn_that_already_ended_says_so_and_reports_background_work() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let conversation = root(&runtime, "top", ".");
    let prompt = running(&runtime, &conversation, "top");
    let (run_id, turn_id) = status_turn(&runtime, &conversation);
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

    let idle = interrupt_wire(&runtime, &conversation, &run_id, &turn_id).await;
    assert_eq!(
        idle,
        serde_json::json!({"type": "error", "body": {
            "code": "interruptTurnEnded",
            "message": "the interrupted turn already ended",
            "interrupt": {
                "conversationId": conversation.0,
                "runId": run_id,
                "turnId": turn_id,
                "turnPhase": "completed",
                "backgroundWorkRunning": false
            }
        }})
    );

    open_child(&runtime, &run_id);
    let background = interrupt_wire(&runtime, &conversation, &run_id, &turn_id).await;
    assert_eq!(
        background["body"]["interrupt"]["backgroundWorkRunning"],
        true
    );

    let stopped = runtime
        .agent_chat_intent(AgentChatIntentFrame::Interrupt {
            request_id: AgentChatRequestId("stop-background".into()),
            receipt_id: ReceiptId("stop-background-receipt".into()),
            conversation_id: conversation.clone(),
            run_id: AgentChatRunId(run_id),
            expected_turn_id: None,
        })
        .unwrap();
    assert!(matches!(
        stopped.as_slice(),
        [AgentChatIntentFrame::Interrupted { .. }]
    ));
}
