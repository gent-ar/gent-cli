use gent_ports::AgentChatSideQuestionLedger;
use gent_protocol::{
    AgentChatProjectionDelta, AgentChatProjectionFrame, AgentChatSideQuestionFrame,
    ProjectionCursor, read_json_frame, write_json_frame,
};
use gent_types::{
    AgentChatConversationId, AgentChatRequestId, AgentChatSideQuestion,
    AgentChatSideQuestionCancel, AgentChatSideQuestionOutcome, HostEpoch, ReceiptId,
};
use serde_json::{Value, json};

use super::{RuntimeFacade, facade, root};
use crate::api::RuntimeApi;

fn ask(runtime: &RuntimeFacade, conversation: &AgentChatConversationId, key: &str) -> String {
    runtime
        .conversation_links()
        .unwrap()
        .ask_agent_chat_side_question(
            &AgentChatSideQuestion {
                request_id: AgentChatRequestId(format!("ask-{key}")),
                receipt_id: ReceiptId(format!("ask-receipt-{key}")),
                host_epoch: HostEpoch(1),
                conversation_id: conversation.clone(),
                question: format!("what about {key}?"),
                created_at_unix_ms: 1,
            },
            &format!("side-{key}"),
        )
        .unwrap()
        .record
        .side_question_id
}

fn settled(runtime: &RuntimeFacade, conversation: &AgentChatConversationId) -> Vec<Value> {
    runtime
        .agent_chat_projection_follow(&conversation.0, &ProjectionCursor { value: 0 })
        .unwrap()
        .into_iter()
        .filter(|delta| matches!(delta, AgentChatProjectionDelta::Activity { .. }))
        .map(|delta| serde_json::to_value(&delta).unwrap()["body"]["fact"].clone())
        .filter(|fact| fact["type"] == "sideQuestionSettled")
        .collect()
}

#[tokio::test]
async fn a_settled_side_question_reaches_followers_once_with_its_answer() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let conversation = root(&runtime, "top", ".");
    let answered = ask(&runtime, &conversation, "answered");
    let cancelled = ask(&runtime, &conversation, "cancelled");
    assert!(settled(&runtime, &conversation).is_empty());

    let ledger = runtime.conversation_links().unwrap();
    let outcome = AgentChatSideQuestionOutcome::Answered {
        text: "it is fine".into(),
    };
    ledger
        .complete_agent_chat_side_question(&answered, &outcome)
        .unwrap();
    let cancel = AgentChatSideQuestionCancel {
        request_id: AgentChatRequestId("cancel".into()),
        receipt_id: ReceiptId("cancel-receipt".into()),
        host_epoch: HostEpoch(1),
        side_question_id: cancelled.clone(),
    };
    ledger.cancel_agent_chat_side_question(&cancel).unwrap();
    ledger.cancel_agent_chat_side_question(&cancel).unwrap();

    let facts = settled(&runtime, &conversation);
    assert_eq!(facts.len(), 2);
    let summary = |fact: &Value| {
        json!({
            "conversationId": fact["conversationId"],
            "sideQuestionId": fact["sideQuestionId"],
            "askReceiptId": fact["askReceiptId"],
            "status": fact["status"],
            "answer": fact["answer"],
            "failureReason": fact["failureReason"],
        })
    };
    assert_eq!(
        facts.iter().map(summary).collect::<Vec<_>>(),
        [
            json!({"conversationId": conversation.0, "sideQuestionId": answered, "askReceiptId": "ask-receipt-answered", "status": "answered", "answer": "it is fine", "failureReason": null}),
            json!({"conversationId": conversation.0, "sideQuestionId": cancelled, "askReceiptId": "ask-receipt-cancelled", "status": "cancelled", "answer": null, "failureReason": null}),
        ]
    );
    assert!(facts.iter().all(|fact| fact["cursor"].as_u64().unwrap() > 0
        && fact["turnId"].as_str().is_some_and(|turn| !turn.is_empty())));
}

#[tokio::test]
async fn a_follower_matches_the_settled_fact_to_the_receipt_it_asked_with() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let conversation = root(&runtime, "top", ".");
    let mut follower = super::projection_window_tests::connect(&runtime).await;
    write_json_frame(
        &mut follower,
        &AgentChatProjectionFrame::FollowConversation {
            request_id: "follow".into(),
            conversation_id: conversation.0.clone(),
            after_cursor: ProjectionCursor { value: 0 },
        },
    )
    .await
    .unwrap();
    let AgentChatSideQuestionFrame::Asked { record, .. } = runtime
        .agent_chat_side_question(AgentChatSideQuestionFrame::AskSideQuestion {
            request_id: "ask-wire".into(),
            receipt_id: "client-ask-receipt".into(),
            conversation_id: conversation.0.clone(),
            question: "what now?".into(),
        })
        .unwrap()
    else {
        panic!("the ask is accepted");
    };
    let fact = loop {
        let AgentChatProjectionFrame::Delta { delta, .. } =
            read_json_frame(&mut follower).await.unwrap()
        else {
            panic!("a follow streams deltas");
        };
        let fact = serde_json::to_value(&delta).unwrap()["body"]["fact"].clone();
        if fact["type"] == "sideQuestionSettled" {
            break fact;
        }
    };
    assert_eq!(
        (&fact["askReceiptId"], &fact["sideQuestionId"]),
        (
            &json!("client-ask-receipt"),
            &json!(record.side_question_id)
        )
    );
}
