#![cfg(unix)]
use std::process::Command;
use std::sync::{Arc, Mutex};

use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AgentChatIntentFrame, DependencyActionResult,
    DependencyActionState, DependencyPlan, Hello, Negotiated, WireFrame, read_frame,
    read_json_frame, write_frame, write_json_frame,
};
use gent_types::{
    AgentChatConversationId, AgentChatPromptDelivery, AgentChatRunId, CapabilitySet, HostEpoch,
    HostStatus, PROTOCOL_MAX, Receipt, ReceiptStatus,
};
use tempfile::TempDir;
use tokio::net::UnixListener;
fn status() -> WireFrame {
    WireFrame::Status(HostStatus {
        host_epoch: HostEpoch(7),
        protocol_min: PROTOCOL_MAX,
        protocol_max: PROTOCOL_MAX,
        capabilities: CapabilitySet(vec!["events".into(), "receipts".into()]),
    })
}
fn server(directory: &TempDir, connections: usize) -> Arc<Mutex<Vec<WireFrame>>> {
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    let received = Arc::new(Mutex::new(Vec::new()));
    let saved = Arc::clone(&received);
    tokio::spawn(async move {
        for _ in 0..connections {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert!(matches!(
                read_frame(&mut stream).await.unwrap(),
                WireFrame::Hello(Hello { capabilities, .. }) if capabilities.0.contains(&"events".into())
            ));
            write_frame(
                &mut stream,
                &WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet(vec!["events".into(), "receipts".into()]),
                }),
            )
            .await
            .unwrap();
            let request = read_frame(&mut stream).await.unwrap();
            saved.lock().unwrap().push(request.clone());
            let response = match request {
                WireFrame::DependencyPlanRequest(ref request) => {
                    WireFrame::DependencyPlan(DependencyPlan::reviewed(
                        request.provider,
                        request.action,
                        "review the vendor installer",
                        true,
                    ))
                }
                WireFrame::DependencyActionRequest(ref request) => {
                    WireFrame::DependencyActionResult(DependencyActionResult {
                        plan: DependencyPlan::reviewed(
                            request.provider,
                            request.action,
                            "review the vendor installer",
                            true,
                        ),
                        state: DependencyActionState::ConsentRequired,
                        receipt: Receipt {
                            receipt_id: request.receipt_id.clone(),
                            idempotency_key: request.idempotency_key.clone(),
                            status: ReceiptStatus::Rejected,
                            host_epoch: request.host_epoch,
                        },
                        detail: None,
                    })
                }
                _ => status(),
            };
            write_frame(&mut stream, &response).await.unwrap();
        }
    });
    received
}

fn run(directory: &TempDir, args: &[&str]) {
    let output = Command::new(env!("CARGO_BIN_EXE_gent"))
        .args([
            "--data-dir",
            directory.path().to_str().unwrap(),
            "--no-autostart",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "gent failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.stdout.is_empty());
}

fn chat_server(directory: &TempDir) {
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert!(matches!(
                read_frame(&mut stream).await.unwrap(),
                WireFrame::Hello(_)
            ));
            write_frame(
                &mut stream,
                &WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet(vec![AGENT_CHAT_INTENTS_CAPABILITY.into()]),
                }),
            )
            .await
            .unwrap();
            let request: AgentChatIntentFrame = read_json_frame(&mut stream).await.unwrap();
            match request {
                AgentChatIntentFrame::CreateConversation {
                    request_id,
                    receipt_id,
                    selection,
                    ..
                } => {
                    assert_eq!(selection.provider, gent_types::AgentChatProvider::Codex);
                    write_json_frame(
                        &mut stream,
                        &AgentChatIntentFrame::Created {
                            request_id,
                            receipt: Receipt {
                                receipt_id,
                                idempotency_key: "create".into(),
                                status: ReceiptStatus::Settled,
                                host_epoch: HostEpoch(7),
                            },
                            conversation_id: AgentChatConversationId("conversation-1".into()),
                            run_id: AgentChatRunId("run-1".into()),
                        },
                    )
                    .await
                    .unwrap();
                }
                AgentChatIntentFrame::SendPrompt {
                    request_id,
                    receipt_id,
                    conversation_id,
                    text,
                } => {
                    assert_eq!(conversation_id.0, "conversation-1");
                    assert_eq!(text, "draft the release notes");
                    write_json_frame(
                        &mut stream,
                        &AgentChatIntentFrame::Accepted {
                            request_id,
                            receipt: Receipt {
                                receipt_id,
                                idempotency_key: "send".into(),
                                status: ReceiptStatus::Settled,
                                host_epoch: HostEpoch(7),
                            },
                            conversation_id: AgentChatConversationId("conversation-1".into()),
                            run_id: AgentChatRunId("run-1".into()),
                            turn_id: "turn-1".into(),
                            delivery: AgentChatPromptDelivery::AwaitingProvider,
                        },
                    )
                    .await
                    .unwrap();
                }
                _ => panic!("direct prompt must not submit another frame"),
            }
        }
    });
}

#[test]
fn cli_prints_its_package_version_without_contacting_a_daemon() {
    let output = Command::new(env!("CARGO_BIN_EXE_gent"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("gent "));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_maps_every_public_command_to_a_negotiated_protocol_frame() {
    let directory = tempfile::tempdir().unwrap();
    let received = server(&directory, 16);
    for args in [
        &["doctor"][..],
        &["onboarding"],
        &["deps", "plan", "install", "claude"],
        &["deps", "install", "codex"],
        &["deps", "update", "claude", "--consent"],
        &["status"],
        &["events", "--after-cursor", "2"],
        &[
            "decision",
            "submit",
            "--decision-id",
            "d1",
            "--idempotency-key",
            "i1",
        ],
        &["decision", "unprovable", "--decision-id", "d1"],
        &["decision", "recovery", "--decision-id", "d1"],
        &[
            "submit",
            "--kind",
            "ping",
            "--payload",
            r#"{"source":"cli-test"}"#,
            "--idempotency-key",
            "submit-key",
        ],
    ] {
        run(&directory, args);
    }
    let received = received.lock().unwrap();
    assert!(matches!(received[0], WireFrame::DoctorRequest));
    assert!(matches!(received[1], WireFrame::OnboardingRequest));
    assert!(matches!(received[2], WireFrame::DependencyPlanRequest(_)));
    assert!(matches!(received[3], WireFrame::DependencyPlanRequest(_)));
    assert!(matches!(received[4], WireFrame::StatusRequest));
    assert!(
        matches!(received[5], WireFrame::DependencyActionRequest(ref request) if !request.consent_granted && request.host_epoch == HostEpoch(7))
    );
    assert!(matches!(received[6], WireFrame::DependencyPlanRequest(_)));
    assert!(matches!(received[7], WireFrame::StatusRequest));
    assert!(
        matches!(received[8], WireFrame::DependencyActionRequest(ref request) if request.consent_granted && request.host_epoch == HostEpoch(7))
    );
    assert!(matches!(received[9], WireFrame::StatusRequest));
    assert!(matches!(
        received[10],
        WireFrame::Subscribe { after_cursor: 2 }
    ));
    assert!(matches!(received[11], WireFrame::DecisionSubmit(_)));
    for frame in &received[12..14] {
        assert!(matches!(frame, WireFrame::DecisionRecovery { .. }));
    }
    assert!(matches!(received[14], WireFrame::StatusRequest));
    assert!(matches!(
        received[15],
        WireFrame::Command(ref command)
            if command.host_epoch == HostEpoch(7)
                && command.idempotency_key == "submit-key"
                && command.kind == "ping"
    ));
}

#[tokio::test]
async fn cli_no_autostart_reports_a_missing_daemon_without_spawning_one() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_gent"))
        .args([
            "--data-dir",
            directory.path().to_str().unwrap(),
            "--no-autostart",
            "status",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--no-autostart"));
    assert!(!directory.path().join("gentd.sock").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn positional_prompt_creates_then_sends_only_typed_agent_chat_intents() {
    let directory = tempfile::tempdir().unwrap();
    chat_server(&directory);
    let output = Command::new(env!("CARGO_BIN_EXE_gent"))
        .args([
            "--data-dir",
            directory.path().to_str().unwrap(),
            "--no-autostart",
            "draft the release notes",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("conversation-1"));
    assert!(stdout.contains("awaitingProvider"));
}
#[test]
fn default_browser_rejects_non_tty_before_daemon_autostart() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_gent"))
        .arg("--data-dir")
        .arg(directory.path())
        .env("GENTD_BIN", directory.path().join("must-not-start"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires a terminal"));
    assert!(!directory.path().join("gentd.sock").exists());
}
