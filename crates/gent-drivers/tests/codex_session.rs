use gent_drivers::codex_session::{
    CodexAppServerSession, CodexSessionConfig, CodexSessionError, CodexSessionIngress,
    CodexTurnOptions,
};
use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection};
use serde_json::{Value, json};
fn config(resume_thread_id: Option<&str>) -> CodexSessionConfig {
    CodexSessionConfig {
        working_directory: Some("/work".into()),
        resume_thread_id: resume_thread_id.map(str::to_owned),
        turn_options: options(AgentChatMode::Agent),
        mcp_servers: None,
    }
}

fn options(mode: AgentChatMode) -> CodexTurnOptions {
    CodexTurnOptions::from_selection(
        &AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "gpt-5.6".into(),
            effort: AgentChatEffort::Medium,
            mode,
        },
        Some("/work"),
    )
    .unwrap()
}

fn decode(frame: &[u8]) -> Value {
    assert_eq!(frame.last(), Some(&b'\n'));
    let mut decoded: Value = serde_json::from_slice(&frame[..frame.len() - 1]).unwrap();
    assert_eq!(decoded["jsonrpc"], json!("2.0"));
    decoded.as_object_mut().unwrap().remove("jsonrpc");
    decoded
}

#[path = "codex_session/default_model.rs"]
mod default_model;
#[path = "codex_session/handshake.rs"]
mod handshake;
#[path = "codex_session/options.rs"]
mod options;
#[path = "codex_session/resume.rs"]
mod resume;
