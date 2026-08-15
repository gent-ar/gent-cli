#![cfg(unix)]

use std::path::PathBuf;
use std::process::{Child, Command as ProcessCommand, Stdio};

use gent_protocol::{
    DependencyAction, DependencyActionRequest, DependencyPlanRequest, DependencyProvider, Hello,
    PublicRunOutcome, PublicRunStartRequest, WireFrame, read_frame, write_frame,
};
use gent_types::{
    CapabilitySet, Command, HostEpoch, McpPermissionStatus, PROTOCOL_MAX, PROTOCOL_MIN, ReceiptId,
};
use serde_json::json;
use tempfile::TempDir;
use tokio::net::UnixStream;

struct Daemon {
    child: Child,
    _directory: TempDir,
    socket: PathBuf,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn daemon() -> Daemon {
    let directory = tempfile::tempdir().unwrap();
    let empty_path = directory.path().join("empty-path");
    std::fs::create_dir(&empty_path).unwrap();
    let mut child = ProcessCommand::new(env!("CARGO_BIN_EXE_gentd"))
        .args(["--data-dir", directory.path().to_str().unwrap()])
        .env("PATH", empty_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let socket = directory.path().join("gentd.sock");
    for _ in 0..40 {
        if UnixStream::connect(&socket).await.is_ok() {
            return Daemon {
                child,
                _directory: directory,
                socket,
            };
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("gentd did not create its local socket");
}

fn hello() -> WireFrame {
    WireFrame::Hello(Hello {
        protocol_min: PROTOCOL_MIN,
        protocol_max: PROTOCOL_MAX,
        capabilities: CapabilitySet(vec![
            "decisions".into(),
            "event-resync".into(),
            "events".into(),
            "host-epoch".into(),
            "receipts".into(),
        ]),
    })
}

async fn client(daemon: &Daemon) -> UnixStream {
    let mut stream = UnixStream::connect(&daemon.socket).await.unwrap();
    write_frame(&mut stream, &hello()).await.unwrap();
    assert!(matches!(
        read_frame(&mut stream).await.unwrap(),
        WireFrame::Negotiated(answer) if answer.protocol == PROTOCOL_MAX
    ));
    stream
}

async fn request(stream: &mut UnixStream, frame: WireFrame) -> WireFrame {
    write_frame(stream, &frame).await.unwrap();
    read_frame(stream).await.unwrap()
}

#[tokio::test]
async fn daemon_requires_hello_and_negotiates_before_status() {
    let daemon = daemon().await;
    let mut unnegotiated = UnixStream::connect(&daemon.socket).await.unwrap();
    write_frame(&mut unnegotiated, &WireFrame::StatusRequest)
        .await
        .unwrap();
    assert!(matches!(
        read_frame(&mut unnegotiated).await.unwrap(),
        WireFrame::Error { code, .. } if code == "handshakeRequired"
    ));

    let mut incompatible = UnixStream::connect(&daemon.socket).await.unwrap();
    write_frame(
        &mut incompatible,
        &WireFrame::Hello(Hello {
            protocol_min: PROTOCOL_MAX + 1,
            protocol_max: PROTOCOL_MAX + 1,
            capabilities: CapabilitySet::default(),
        }),
    )
    .await
    .unwrap();
    assert!(matches!(
        read_frame(&mut incompatible).await.unwrap(),
        WireFrame::Error { code, .. } if code == "upgradeRequired"
    ));

    let mut stream = client(&daemon).await;
    assert!(matches!(
        request(&mut stream, WireFrame::StatusRequest).await,
        WireFrame::Status(status) if status.host_epoch == HostEpoch(1)
    ));
}

#[tokio::test]
async fn command_receipts_are_idempotent_and_events_resume_over_ipc() {
    let daemon = daemon().await;
    let mut stream = client(&daemon).await;
    let command = Command {
        receipt_id: ReceiptId("receipt-1".into()),
        idempotency_key: "idempotency-1".into(),
        host_epoch: HostEpoch(1),
        kind: "ping".into(),
        payload: json!({"source": "ipc-test"}),
    };
    let first = request(&mut stream, WireFrame::Command(command.clone())).await;
    let second = request(&mut stream, WireFrame::Command(command)).await;
    assert_eq!(first, second);
    assert!(
        matches!(first, WireFrame::Receipt(receipt) if receipt.idempotency_key == "idempotency-1")
    );
    assert!(matches!(
        request(&mut stream, WireFrame::Subscribe { after_cursor: 0 }).await,
        WireFrame::Events { events }
            if events.len() == 2
                && events[0].kind == "commandAccepted"
                && events[1].kind == "commandSettled"
    ));
}

#[tokio::test]
async fn observer_daemon_exposes_only_read_only_doctor_and_dependency_plans() {
    let daemon = daemon().await;
    let mut stream = client(&daemon).await;
    assert!(matches!(
        request(&mut stream, WireFrame::DoctorRequest).await,
        WireFrame::DoctorReport(report)
            if report.mcp.permission == McpPermissionStatus::HardDisabledObserver
                && report.public_providers.len() == 2
    ));
    assert!(matches!(
        request(
            &mut stream,
            WireFrame::DependencyPlanRequest(DependencyPlanRequest {
                provider: DependencyProvider::Claude,
                action: DependencyAction::Install,
            }),
        )
        .await,
        WireFrame::DependencyPlan(plan) if plan.consent_required
    ));
    assert!(matches!(
        request(
            &mut stream,
            WireFrame::DependencyActionRequest(DependencyActionRequest {
                provider: DependencyProvider::Codex,
                action: DependencyAction::Update,
                consent_granted: false,
            }),
        )
        .await,
        WireFrame::DependencyActionResult(result)
            if result.state == gent_protocol::DependencyActionState::ConsentRequired
    ));
    assert!(matches!(
        request(
            &mut stream,
            WireFrame::PublicRunStart(PublicRunStartRequest {
                run_id: "run-1".into(),
                coordinator_id: "ipc-test".into(),
                host_epoch: HostEpoch(1),
                provider: DependencyProvider::Claude,
                executable: "/public/claude".into(),
                version: "1".into(),
                compatibility_entry: "fixture".into(),
            }),
        )
        .await,
        WireFrame::PublicRunResponse(response) if response.outcome == PublicRunOutcome::Denied
    ));
}
