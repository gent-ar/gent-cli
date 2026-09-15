use gent_types::{
    AgentChatCommandAvailability, AgentChatCommandDescriptor, AgentChatCommandDispatch,
    AgentChatCommandOrigin, AgentChatConversationId, AgentChatProvider, AgentChatRequestId,
    ReceiptId,
};
use serde_json::json;

use super::{
    AgentChatCommandFrame, AgentChatCommandFrameError, CommandCatalog, CommandCatalogScope,
    CommandListing,
};

fn command(name: &str, aliases: &[&str]) -> AgentChatCommandDescriptor {
    AgentChatCommandDescriptor {
        name: name.into(),
        aliases: aliases.iter().map(|alias| (*alias).into()).collect(),
        description: "Show context usage".into(),
        argument_hint: None,
        origin: AgentChatCommandOrigin::ProviderBuiltin {
            provider: AgentChatProvider::Claude,
        },
        dispatch: AgentChatCommandDispatch::ProviderNative,
        availability: AgentChatCommandAvailability {
            requires_conversation: true,
            blocked_while_turn_active: true,
        },
    }
}

fn catalog(commands: Vec<AgentChatCommandDescriptor>) -> AgentChatCommandFrame {
    AgentChatCommandFrame::CommandCatalog {
        request_id: AgentChatRequestId("catalog-1".into()),
        catalog: CommandCatalog {
            scope: CommandCatalogScope {
                conversation_id: Some(AgentChatConversationId("conversation-1".into())),
                workspace_path: None,
                provider: Some(AgentChatProvider::Claude),
            },
            revision: "sha256:abc".into(),
            provider_version: None,
            listing: CommandListing::Ready,
            commands,
        },
    }
}

#[test]
fn invoke_command_is_receipt_bound_and_camel_cased() {
    let frame = AgentChatCommandFrame::InvokeCommand {
        request_id: AgentChatRequestId("request-1".into()),
        receipt_id: ReceiptId("receipt-1".into()),
        conversation_id: Some(AgentChatConversationId("conversation-1".into())),
        workspace_path: None,
        name: "model".into(),
        arguments: "opus".into(),
    };
    assert_eq!(
        serde_json::to_value(&frame).unwrap(),
        json!({"type": "invokeCommand", "body": {
            "requestId": "request-1", "receiptId": "receipt-1",
            "conversationId": "conversation-1", "workspacePath": null,
            "name": "model", "arguments": "opus"
        }})
    );
    assert!(frame.validate().is_ok() && frame.is_client_request());
}

#[test]
fn invalid_names_and_duplicate_catalog_names_are_rejected() {
    let invoke = json!({"type": "invokeCommand", "body": {
        "requestId": "request-1", "receiptId": "receipt-1", "conversationId": null,
        "workspacePath": null, "name": "/model", "arguments": ""
    }});
    let frame: AgentChatCommandFrame = serde_json::from_value(invoke).unwrap();
    assert_eq!(
        frame.validate(),
        Err(AgentChatCommandFrameError::InvalidInvocation)
    );
    assert!(
        catalog(vec![command("context", &[]), command("usage", &["cost"])])
            .validate()
            .is_ok()
    );
    assert_eq!(
        catalog(vec![
            command("context", &[]),
            command("usage", &["context"])
        ])
        .validate(),
        Err(AgentChatCommandFrameError::InvalidCatalog)
    );
    assert!(!catalog(Vec::new()).is_client_request());
}

#[test]
fn catalog_frames_reject_unknown_fields() {
    let read = json!({"type": "readCommandCatalog", "body": {
        "requestId": "catalog-1", "conversationId": null, "workspacePath": "/work",
        "refresh": false, "provider": "claude"
    }});
    assert!(serde_json::from_value::<AgentChatCommandFrame>(read).is_err());
}
