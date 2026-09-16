use gent_protocol::AgentChatIntentFrame;
use gent_types::{AgentChatRequestId, ConversationMessageDelivery, DurableTurnPhase, ReceiptId};

use super::{child, facade, facts, root, running};
use crate::api::RuntimeApi;

#[tokio::test]
async fn a_child_inherits_its_parents_workspace_and_selection_and_is_told_who_created_it() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let top = root(&runtime, "top", &workspace.path().display().to_string());
    let planner = child(&runtime, "planner", &top, "the planner");
    let worker = child(&runtime, "worker", &planner, "the worker");

    let detail = runtime.agent_chat_reads.as_ref().unwrap();
    let inherited = detail.detail(&worker.0).unwrap().summary;
    let parent = detail.detail(&planner.0).unwrap().summary;
    assert_eq!(inherited.selection, parent.selection);
    assert_eq!(inherited.workspace_path, parent.workspace_path);
    assert_eq!(
        inherited.workspace_path,
        Some(
            std::fs::canonicalize(workspace.path())
                .unwrap()
                .display()
                .to_string()
        )
    );

    let created = &facts(&runtime, &planner)[1];
    assert_eq!(created["type"], "conversationCreated");
    assert_eq!(created["childConversationId"], worker.0.as_str());
    assert_eq!(created["label"], "the worker");
    assert_eq!(created["originToolUseId"], "tool-worker");

    let created_by = &facts(&runtime, &worker)[0];
    assert_eq!(created_by["type"], "createdByConversation");
    assert_eq!(created_by["parentConversationId"], planner.0.as_str());
    assert_eq!(created_by["label"], "the planner");
}

#[tokio::test]
async fn addressing_an_idle_conversation_queues_one_prompt_and_names_the_target() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let top = root(&runtime, "top", ".");
    let worker = child(&runtime, "worker", &top, "the worker");

    let delivered = runtime
        .agent_chat_intent(AgentChatIntentFrame::SendToConversation {
            request_id: AgentChatRequestId("send-1".into()),
            receipt_id: ReceiptId("send-receipt-1".into()),
            from_conversation_id: top.clone(),
            target_conversation_id: worker.clone(),
            message: "start on the parser".into(),
        })
        .unwrap();
    let [
        AgentChatIntentFrame::ConversationMessageDelivered {
            delivery,
            message_id,
            ..
        },
    ] = delivered.as_slice()
    else {
        panic!("one delivery is returned")
    };
    assert_eq!(*delivery, ConversationMessageDelivery::Queued);
    assert!(!message_id.is_empty());

    let sent = facts(&runtime, &top).pop().unwrap();
    assert_eq!(sent["type"], "conversationMessageSent");
    assert_eq!(sent["targetConversationId"], worker.0.as_str());
    assert_eq!(sent["targetLabel"], "the worker");
    assert_eq!(sent["delivery"], "queued");
    assert_eq!(sent["preview"], "start on the parser");
}

#[tokio::test]
async fn addressing_a_working_conversation_steers_the_turn_in_flight() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let top = root(&runtime, "top", ".");
    let worker = child(&runtime, "worker", &top, "the worker");
    running(&runtime, &worker, "worker");

    let delivered = runtime
        .agent_chat_intent(AgentChatIntentFrame::SendToConversation {
            request_id: AgentChatRequestId("send-2".into()),
            receipt_id: ReceiptId("send-receipt-2".into()),
            from_conversation_id: top.clone(),
            target_conversation_id: worker,
            message: "also fix the lexer".into(),
        })
        .unwrap();
    assert!(matches!(
        delivered.as_slice(),
        [AgentChatIntentFrame::ConversationMessageDelivered { delivery, .. }]
            if *delivery == ConversationMessageDelivery::Steered
    ));
    assert_eq!(facts(&runtime, &top).pop().unwrap()["delivery"], "steered");
}

#[tokio::test]
async fn a_bounded_wait_reports_an_honest_partial_result_rather_than_failing() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let top = root(&runtime, "top", ".");
    let worker = child(&runtime, "worker", &top, "the worker");
    running(&runtime, &worker, "worker");

    let settled = runtime
        .agent_chat_intent(AgentChatIntentFrame::WaitForConversations {
            request_id: AgentChatRequestId("wait-1".into()),
            from_conversation_id: top.clone(),
            conversation_ids: vec![worker.clone()],
            timeout_seconds: 0,
        })
        .unwrap();
    let [
        AgentChatIntentFrame::ConversationWaitSettled {
            results, timed_out, ..
        },
    ] = settled.as_slice()
    else {
        panic!("one wait result is returned")
    };
    assert!(timed_out);
    assert_eq!(results[0].target.conversation_id, worker.0);
    assert_eq!(results[0].target.label, "the worker");
    assert_eq!(results[0].target.phase, Some(DurableTurnPhase::Active));

    let fact = facts(&runtime, &top).pop().unwrap();
    assert_eq!(fact["type"], "conversationWaitSettled");
    assert_eq!(fact["timedOut"], true);
    assert_eq!(fact["targets"][0]["label"], "the worker");
}

#[tokio::test]
async fn a_settled_conversation_ends_the_wait_and_returns_its_last_reply() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let top = root(&runtime, "top", ".");
    let worker = child(&runtime, "worker", &top, "the worker");

    let settled = runtime
        .agent_chat_intent(AgentChatIntentFrame::WaitForConversations {
            request_id: AgentChatRequestId("wait-2".into()),
            from_conversation_id: top,
            conversation_ids: vec![worker],
            timeout_seconds: 5,
        })
        .unwrap();
    assert!(matches!(
        settled.as_slice(),
        [AgentChatIntentFrame::ConversationWaitSettled { results, timed_out, .. }]
            if !timed_out && results[0].target.is_settled()
    ));
}

#[tokio::test]
async fn listing_a_thread_returns_the_parent_and_every_child_with_its_label() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let top = root(&runtime, "top", ".");
    let planner = child(&runtime, "planner", &top, "the planner");
    let worker = child(&runtime, "worker", &planner, "the worker");
    let reviewer = child(&runtime, "reviewer", &planner, "the reviewer");

    let listed = runtime
        .agent_chat_intent(AgentChatIntentFrame::ListLinkedConversations {
            request_id: AgentChatRequestId("list-1".into()),
            conversation_id: planner.clone(),
        })
        .unwrap();
    let [AgentChatIntentFrame::ConversationLinks { links, .. }] = listed.as_slice() else {
        panic!("one thread is returned")
    };
    let parent = links.parent.as_ref().unwrap();
    assert_eq!(parent.conversation_id, top.0);
    assert_eq!(parent.label, "the planner");
    assert_eq!(
        links
            .children
            .iter()
            .map(|child| (child.conversation_id.clone(), child.label.clone()))
            .collect::<Vec<_>>(),
        vec![
            (worker.0.clone(), "the worker".to_owned()),
            (reviewer.0.clone(), "the reviewer".to_owned()),
        ]
    );
}
