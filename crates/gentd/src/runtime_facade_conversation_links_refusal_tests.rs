use gent_protocol::AgentChatIntentFrame;
use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRequestId,
    AgentChatSelection, ConversationMessageDelivery, ReceiptId,
};

use super::{child, facade, facts, root, root_with_selection};
use crate::{api::RuntimeApi, runtime_facade::RuntimeFacade};

fn send(
    runtime: &RuntimeFacade,
    key: &str,
    from: &AgentChatConversationId,
    target: &AgentChatConversationId,
) -> Result<Vec<AgentChatIntentFrame>, crate::agent_chat_intent_error::AgentChatIntentError> {
    runtime.agent_chat_intent(AgentChatIntentFrame::SendToConversation {
        request_id: AgentChatRequestId(format!("send-{key}")),
        receipt_id: ReceiptId(format!("send-receipt-{key}")),
        from_conversation_id: from.clone(),
        target_conversation_id: target.clone(),
        message: "hello".into(),
    })
}

fn wait(
    runtime: &RuntimeFacade,
    key: &str,
    from: &AgentChatConversationId,
    targets: Vec<AgentChatConversationId>,
) -> Result<Vec<AgentChatIntentFrame>, crate::agent_chat_intent_error::AgentChatIntentError> {
    runtime.agent_chat_intent(AgentChatIntentFrame::WaitForConversations {
        request_id: AgentChatRequestId(format!("wait-{key}")),
        from_conversation_id: from.clone(),
        conversation_ids: targets,
        timeout_seconds: 1,
    })
}

#[tokio::test]
async fn a_conversation_may_never_address_or_await_itself() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let top = root(&runtime, "top", ".");

    assert_eq!(
        send(&runtime, "self", &top, &top).unwrap_err().code,
        "conversationSelfTargeted"
    );
    assert_eq!(
        wait(&runtime, "self", &top, vec![top.clone()])
            .unwrap_err()
            .code,
        "conversationSelfTargeted"
    );
    assert!(facts(&runtime, &top).is_empty());
}

#[tokio::test]
async fn an_unknown_conversation_is_a_typed_refusal_rather_than_a_wait() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let top = root(&runtime, "top", ".");
    let missing = AgentChatConversationId("conversation-nowhere".into());

    for code in [
        send(&runtime, "missing", &top, &missing).unwrap_err().code,
        wait(&runtime, "missing", &top, vec![missing.clone()])
            .unwrap_err()
            .code,
        runtime
            .agent_chat_intent(AgentChatIntentFrame::ListLinkedConversations {
                request_id: AgentChatRequestId("list-missing".into()),
                conversation_id: missing.clone(),
            })
            .unwrap_err()
            .code,
        runtime
            .agent_chat_intent(AgentChatIntentFrame::CreateLinkedConversation {
                request_id: AgentChatRequestId("linked-missing".into()),
                receipt_id: ReceiptId("linked-receipt-missing".into()),
                parent_conversation_id: missing,
                workspace_path: None,
                selection: None,
                label: "the worker".into(),
                prompt: None,
                origin_tool_use_id: None,
            })
            .unwrap_err()
            .code,
    ] {
        assert_eq!(code, "conversationNotFound");
    }
}

#[tokio::test]
async fn a_conversation_may_never_address_another_workspace() {
    let directory = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let here = root(&runtime, "here", ".");
    let there = root(&runtime, "there", &elsewhere.path().display().to_string());

    assert_eq!(
        send(&runtime, "foreign", &here, &there).unwrap_err().code,
        "conversationForeignWorkspace"
    );
    assert_eq!(
        wait(&runtime, "foreign", &here, vec![there])
            .unwrap_err()
            .code,
        "conversationForeignWorkspace"
    );
    assert!(facts(&runtime, &here).is_empty());
}

#[tokio::test]
async fn an_unlabeled_child_or_an_empty_message_is_refused_before_anything_is_created() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let top = root(&runtime, "top", ".");
    let worker = child(&runtime, "worker", &top, "the worker");

    assert_eq!(
        runtime
            .agent_chat_intent(AgentChatIntentFrame::CreateLinkedConversation {
                request_id: AgentChatRequestId("linked-blank".into()),
                receipt_id: ReceiptId("linked-receipt-blank".into()),
                parent_conversation_id: top.clone(),
                workspace_path: None,
                selection: None,
                label: "   ".into(),
                prompt: None,
                origin_tool_use_id: None,
            })
            .unwrap_err()
            .code,
        "conversationLabelInvalid"
    );
    assert_eq!(
        runtime
            .agent_chat_intent(AgentChatIntentFrame::SendToConversation {
                request_id: AgentChatRequestId("send-blank".into()),
                receipt_id: ReceiptId("send-receipt-blank".into()),
                from_conversation_id: top.clone(),
                target_conversation_id: worker,
                message: "  ".into(),
            })
            .unwrap_err()
            .code,
        "conversationMessageEmpty"
    );
    assert_eq!(
        wait(&runtime, "none", &top, Vec::new()).unwrap_err().code,
        "conversationWaitTargetsInvalid"
    );
    assert_eq!(facts(&runtime, &top).len(), 1);
}

#[tokio::test]
async fn every_provider_receives_a_cross_conversation_message_through_the_one_prompt_queue() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let top = root(&runtime, "top", ".");

    for (key, provider, model) in [
        ("claude", AgentChatProvider::Claude, "sonnet"),
        ("codex", AgentChatProvider::Codex, "gpt-5.6"),
        ("claurst", AgentChatProvider::Claurst, "qwen3-1-7b-q4-k-m"),
    ] {
        let target = root_with_selection(
            &runtime,
            key,
            ".",
            AgentChatSelection {
                provider,
                model: model.into(),
                effort: AgentChatEffort::Medium,
                mode: AgentChatMode::Agent,
            },
        );
        let delivered = send(&runtime, key, &top, &target).unwrap();
        let [
            AgentChatIntentFrame::ConversationMessageDelivered {
                delivery,
                message_id,
                ..
            },
        ] = delivered.as_slice()
        else {
            panic!("{key} must receive one delivery")
        };
        assert_eq!(*delivery, ConversationMessageDelivery::Queued);
        assert!(!message_id.is_empty());
        let transcript = runtime
            .agent_chat_reads
            .as_ref()
            .unwrap()
            .transcript(&target.0, Some(0), 16)
            .unwrap();
        assert!(
            transcript
                .events
                .iter()
                .any(|event| event.text.contains("hello")),
            "{key} must record the delivered message on the one durable prompt queue"
        );
        assert_eq!(
            facts(&runtime, &top).pop().unwrap()["targetConversationId"],
            target.0.as_str()
        );
    }
}

#[tokio::test]
async fn a_client_that_knows_none_of_the_link_facts_still_restores_the_conversation() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let top = root(&runtime, "top", ".");
    let worker = child(&runtime, "worker", &top, "the worker");
    send(&runtime, "one", &top, &worker).unwrap();
    wait(&runtime, "one", &top, vec![worker]).unwrap();

    let known = [
        "started",
        "thinking",
        "command",
        "subagent",
        "decision",
        "interruption",
        "recovered",
        "terminal",
        "goalUpdated",
        "planUpdated",
    ];
    let restored = facts(&runtime, &top);
    assert_eq!(restored.len(), 3);
    for fact in restored {
        let kind = fact["type"].as_str().unwrap();
        assert!(!known.contains(&kind));
        assert_eq!(fact["conversationId"], top.0.as_str());
        assert!(fact["runId"].is_string());
        assert!(fact["turnId"].is_string());
        assert!(fact["cursor"].is_u64());
    }
    let detail = runtime
        .agent_chat_reads
        .as_ref()
        .unwrap()
        .detail(&top.0)
        .unwrap();
    assert_eq!(detail.summary.conversation_id, top.0);
}

#[tokio::test]
async fn a_child_created_with_a_prompt_starts_that_work_on_the_one_queue() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let top = root(&runtime, "top", ".");
    let Some(AgentChatIntentFrame::LinkedConversationCreated {
        conversation_id: worker,
        ..
    }) = runtime
        .agent_chat_intent(AgentChatIntentFrame::CreateLinkedConversation {
            request_id: AgentChatRequestId("linked-prompted".into()),
            receipt_id: ReceiptId("linked-receipt-prompted".into()),
            parent_conversation_id: top,
            workspace_path: None,
            selection: None,
            label: "the worker".into(),
            prompt: Some("build the parser".into()),
            origin_tool_use_id: None,
        })
        .unwrap()
        .into_iter()
        .next()
    else {
        panic!("a prompted child must be created")
    };

    let transcript = runtime
        .agent_chat_reads
        .as_ref()
        .unwrap()
        .transcript(&worker.0, Some(0), 16)
        .unwrap();
    assert!(
        transcript
            .events
            .iter()
            .any(|event| event.text.contains("build the parser"))
    );
}
