use super::{call, tools};
use serde_json::json;

#[test]
fn every_chat_tool_explains_that_a_created_chat_is_a_real_conversation() {
    let names = tools()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap_or_default().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "gent_chat_create",
            "gent_chat_send",
            "gent_chat_wait",
            "gent_chat_list"
        ]
    );
    let create = tools()[0]["description"].as_str().unwrap().to_owned();
    assert!(create.contains("real conversation the user can open"));
    assert!(create.contains("not a sub-agent"));
    let send = tools()[1]["description"].as_str().unwrap().to_owned();
    assert!(send.contains("only way to talk to it"));
    let wait = tools()[2]["description"].as_str().unwrap().to_owned();
    assert!(wait.contains("Block until"));
    assert!(wait.contains("does not return while they are still working"));
}

#[test]
fn no_chat_tool_requires_the_caller_to_name_its_own_conversation() {
    for tool in tools() {
        let required = tool["inputSchema"]["required"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(
            !required.iter().any(|value| value == "conversationId"),
            "{} must not require conversationId",
            tool["name"]
        );
    }
}

#[tokio::test]
async fn a_tool_call_without_a_bound_conversation_is_refused_before_any_ipc() {
    let error = call(None, true, None, "gent_chat_list", &json!({}))
        .await
        .expect_err("an unbound chat server cannot act");
    assert!(error.to_string().contains("pass conversationId"));
}

#[tokio::test]
async fn a_wait_rejects_conversation_identifiers_that_are_not_strings() {
    let error = call(
        None,
        true,
        Some("conversation-1"),
        "gent_chat_wait",
        &json!({ "conversationIds": [7] }),
    )
    .await
    .expect_err("only string identifiers are valid");
    assert!(error.to_string().contains("only strings"));
}
