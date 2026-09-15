#![cfg(unix)]

use std::process::Command;

use gent_protocol::{
    Hello, Negotiated, WireFrame,
    agent_chat_commands::{AGENT_CHAT_COMMANDS_CAPABILITY, AgentChatCommandFrame, CommandOutcome},
    read_frame, read_json_frame, write_frame, write_json_frame,
};
use gent_types::{
    AgentChatCommandIntent, AgentChatConversationId, AgentChatRunId, CapabilitySet, HostEpoch,
    PROTOCOL_MAX, Receipt, ReceiptStatus,
};
use tokio::net::UnixListener;

async fn answer(
    listener: &UnixListener,
    reply: impl FnOnce(AgentChatCommandFrame) -> serde_json::Value + Send + 'static,
) {
    let (mut stream, _) = listener.accept().await.unwrap();
    assert!(matches!(
        read_frame(&mut stream).await.unwrap(),
        WireFrame::Hello(Hello { capabilities, .. })
            if capabilities.0.contains(&AGENT_CHAT_COMMANDS_CAPABILITY.to_owned())
    ));
    write_frame(
        &mut stream,
        &WireFrame::Negotiated(Negotiated {
            protocol: PROTOCOL_MAX,
            capabilities: CapabilitySet(vec![AGENT_CHAT_COMMANDS_CAPABILITY.into()]),
        }),
    )
    .await
    .unwrap();
    let request: AgentChatCommandFrame = read_json_frame(&mut stream).await.unwrap();
    write_json_frame(&mut stream, &reply(request))
        .await
        .unwrap();
}

fn gent(directory: &std::path::Path, arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_gent"))
        .args(["--data-dir", directory.to_str().unwrap(), "--no-autostart"])
        .args(arguments)
        .output()
        .unwrap()
}

fn direct(directory: &std::path::Path, prompt: &str) -> std::process::Output {
    gent(
        directory,
        &["--conversation-id", "conversation-1", "--json", prompt],
    )
}

fn fork_applied(
    request: AgentChatCommandFrame,
    expected_receipt: Option<&str>,
) -> serde_json::Value {
    let AgentChatCommandFrame::InvokeCommand {
        request_id,
        receipt_id,
        conversation_id,
        name,
        arguments,
        ..
    } = request
    else {
        panic!("a slash prompt must be an InvokeCommand, never a SendPrompt");
    };
    assert_eq!((name.as_str(), arguments.as_str()), ("fork", ""));
    if let Some(expected) = expected_receipt {
        assert_eq!(receipt_id.0, expected);
    }
    serde_json::to_value(AgentChatCommandFrame::CommandInvoked {
        request_id,
        receipt: Receipt {
            receipt_id,
            idempotency_key: "fork".into(),
            status: ReceiptStatus::Settled,
            host_epoch: HostEpoch(1),
        },
        conversation_id,
        outcome: CommandOutcome::IntentApplied {
            intent: AgentChatCommandIntent::ForkConversation,
            conversation_id: AgentChatConversationId("conversation-2".into()),
            run_id: Some(AgentChatRunId("run-2".into())),
        },
    })
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slash_prompt_is_invoked_as_a_command_and_its_outcome_is_printed() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    let server =
        tokio::spawn(async move { answer(&listener, |request| fork_applied(request, None)).await });
    let output = direct(directory.path(), "/fork");
    server.await.unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("forkConversation") && stdout.contains("conversation-2"),
        "{stdout}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_slash_command_reports_gentds_typed_rejection() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    let server = tokio::spawn(async move {
        answer(&listener, |_| {
            serde_json::to_value(WireFrame::Error {
                code: "unknownCommand".into(),
                message: "/nope is not a command in this conversation's catalog".into(),
            })
            .unwrap()
        })
        .await
    });
    let output = direct(directory.path(), "/nope please");
    server.await.unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknownCommand"), "{stderr}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_resume_invokes_a_slash_prompt_under_its_receipt_and_chat_send_refuses_attachments() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    let server = tokio::spawn(async move {
        answer(&listener, |request| {
            fork_applied(request, Some("receipt-7"))
        })
        .await
    });
    let output = gent(
        directory.path(),
        &[
            "chat",
            "resume",
            "conversation-1",
            "/fork",
            "--receipt-id",
            "receipt-7",
            "--json",
        ],
    );
    server.await.unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("conversation-2"));
    let refused = gent(
        directory.path(),
        &[
            "chat",
            "send",
            "--conversation-id",
            "conversation-1",
            "--text",
            "/compact",
            "--attach",
            "notes.md",
        ],
    );
    assert!(!refused.status.success());
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.contains("/compact does not take attachments"),
        "{stderr}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_one_shot_command_waits_for_a_loading_catalog_and_retries_under_the_same_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    let server = tokio::spawn(async move {
        let first = std::sync::Arc::new(std::sync::Mutex::new(None));
        let recorded = first.clone();
        answer(&listener, move |request| {
            let AgentChatCommandFrame::InvokeCommand { receipt_id, .. } = request else {
                panic!("expected InvokeCommand");
            };
            *recorded.lock().unwrap() = Some(receipt_id.0);
            serde_json::to_value(WireFrame::Error {
                code: "commandCatalogLoading".into(),
                message: "the provider's commands are still loading; try /fork again shortly"
                    .into(),
            })
            .unwrap()
        })
        .await;
        let receipt = first.lock().unwrap().clone().unwrap();
        answer(&listener, move |request| {
            fork_applied(request, Some(&receipt))
        })
        .await;
    });
    let output = direct(directory.path(), "/fork");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.await.unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("conversation-2"));
}
