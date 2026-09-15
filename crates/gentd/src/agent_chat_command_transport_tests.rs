use gent_protocol::{
    WireFrame,
    agent_chat_commands::{AGENT_CHAT_COMMANDS_CAPABILITY, AgentChatCommandFrame},
    read_frame, read_json_frame,
};
use gent_runtime::catalog::{RuntimeCapabilityFeature, RuntimeCapabilityProfile};
use gent_types::{AgentChatCommandOrigin, CapabilitySet};
use serde_json::json;
use tokio::io::duplex;

use super::dispatch;
use crate::{CompatibilityAssessment, build_runtime};

fn chat_runtime(directory: &std::path::Path) -> crate::RuntimeFacade {
    build_runtime(
        directory,
        &RuntimeCapabilityProfile::new([RuntimeCapabilityFeature::AgentChat]),
        CompatibilityAssessment::default(),
    )
    .unwrap()
}

fn read_catalog() -> serde_json::Value {
    json!({"type": "readCommandCatalog", "body": {
        "requestId": "catalog-1", "conversationId": null, "workspacePath": "/work", "refresh": false
    }})
}

#[tokio::test]
async fn commands_are_unreachable_without_the_negotiated_capability() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = chat_runtime(directory.path());
    let (_, mut writer) = duplex(64 * 1024);
    assert!(
        !dispatch(
            &mut writer,
            &runtime,
            &CapabilitySet::default(),
            &read_catalog()
        )
        .await
        .unwrap()
    );
    assert!(
        crate::transport::observed_capabilities(&RuntimeCapabilityProfile::new([
            RuntimeCapabilityFeature::AgentChat
        ]))
        .0
        .contains(&AGENT_CHAT_COMMANDS_CAPABILITY.into())
    );
}

#[tokio::test]
async fn a_catalog_read_and_an_unknown_invocation_answer_on_the_wire() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = chat_runtime(directory.path());
    let capabilities = CapabilitySet(vec![AGENT_CHAT_COMMANDS_CAPABILITY.into()]);
    let (mut reader, mut writer) = duplex(256 * 1024);
    assert!(
        dispatch(&mut writer, &runtime, &capabilities, &read_catalog())
            .await
            .unwrap()
    );
    let AgentChatCommandFrame::CommandCatalog { catalog, .. } =
        read_json_frame(&mut reader).await.unwrap()
    else {
        panic!("expected a command catalog");
    };
    assert!(
        catalog
            .commands
            .iter()
            .any(|command| command.name == "resume")
    );
    assert!(
        catalog
            .commands
            .iter()
            .all(|command| command.origin == AgentChatCommandOrigin::Gent)
    );
    let invoke = json!({"type": "invokeCommand", "body": {
        "requestId": "invoke-1", "receiptId": "receipt-1", "conversationId": null,
        "workspacePath": "/work", "name": "nope", "arguments": ""
    }});
    assert!(
        dispatch(&mut writer, &runtime, &capabilities, &invoke)
            .await
            .unwrap()
    );
    let WireFrame::Error { code, message } = read_frame(&mut reader).await.unwrap() else {
        panic!("expected a typed rejection");
    };
    assert_eq!(
        (code.as_str(), message.as_str()),
        (
            "unknownCommand",
            "/nope is not a command in this conversation's catalog"
        )
    );
}
