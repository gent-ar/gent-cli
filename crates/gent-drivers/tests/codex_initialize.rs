use std::path::Path;

use gent_drivers::codex_session::{CodexAppServerSession, CodexSessionConfig, CodexTurnOptions};
use gent_drivers::message_encoding::encode_codex_handshake;
use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection};
use serde_json::{Value, json};

fn pinned_initialize_request() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "clientInfo": {"name": "gent", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"experimentalApi": true, "requestAttestation": false}
        }
    })
}

fn decode(frame: &[u8]) -> Value {
    assert_eq!(frame.last(), Some(&b'\n'));
    serde_json::from_slice(&frame[..frame.len() - 1]).unwrap()
}

fn conversation_config() -> CodexSessionConfig {
    CodexSessionConfig {
        working_directory: Some("/work".into()),
        resume_thread_id: None,
        turn_options: CodexTurnOptions::from_selection(
            &AgentChatSelection {
                provider: AgentChatProvider::Codex,
                model: "gpt-5.6".into(),
                effort: AgentChatEffort::Medium,
                mode: AgentChatMode::Agent,
            },
            Some("/work"),
        )
        .unwrap(),
        mcp_servers: None,
    }
}

fn pinned_codex_initialize_params() -> Value {
    let contracts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/provider-contracts");
    let read = |path: &Path| -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    };
    let pins = read(&contracts.join("pins.json"));
    let version = pins["providers"]["codex"]["version"].as_str().unwrap();
    let protocol = read(&contracts.join("codex").join(version).join("protocol.json"));
    protocol["methods"]["initialize"]["params"].clone()
}

#[test]
fn conversations_and_model_catalog_open_codex_with_one_initialize_request() {
    let (_, conversation) = CodexAppServerSession::start(conversation_config()).unwrap();
    let catalog = encode_codex_handshake(1).unwrap();

    assert_eq!(decode(&conversation), pinned_initialize_request());
    assert_eq!(decode(&catalog[0]), pinned_initialize_request());
}

#[test]
fn initialize_request_fields_exist_in_the_pinned_codex_schema() {
    let schema = pinned_codex_initialize_params();
    let request = pinned_initialize_request();
    let capabilities = request["params"]["capabilities"].as_object().unwrap();

    for field in ["/clientInfo/name", "/clientInfo/version"] {
        assert_eq!(schema[field], json!("string"), "{field}");
    }
    for (capability, value) in capabilities {
        assert!(value.is_boolean(), "{capability}");
        assert_eq!(
            schema[format!("/capabilities/{capability}")],
            json!("boolean"),
            "{capability}"
        );
    }
}
