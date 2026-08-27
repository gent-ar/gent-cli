#![cfg(unix)]

use std::path::PathBuf;
use std::process::{Child, Command as ProcessCommand, Stdio};

use gent_protocol::{
    ATTACHMENTS_CAPABILITY, AttachmentFrame, DependencyAction, DependencyPlanRequest,
    DependencyProvider, Hello, PublicRunOutcome, PublicRunStartRequest, WireFrame, read_frame,
    read_json_frame, write_frame, write_json_frame,
};
use gent_types::{
    AttachmentMetadata, AttachmentOperation, AttachmentState, AttachmentTransfer, CapabilitySet,
    Command, HostEpoch, McpPermissionStatus, PROTOCOL_MAX, PROTOCOL_MIN, ReceiptId,
};
use serde_json::json;
use tempfile::TempDir;
use tokio::net::UnixStream;

struct Daemon {
    child: Child,
    _directory: TempDir,
    socket: PathBuf,
}

#[test]
fn daemon_prints_its_package_version_without_opening_ipc() {
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_gentd"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("gentd "));
}

#[test]
fn daemon_prints_its_resolved_data_dir_without_binding_ipc_or_the_host_lock() {
    let directory = tempfile::tempdir().unwrap();
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_gentd"))
        .args(["--print-data-dir", "--data-dir"])
        .arg(directory.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        directory.path().to_str().unwrap()
    );
    assert!(!directory.path().join("gentd.sock").exists());
    assert!(!directory.path().join("gentd.lock").exists());
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
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        if UnixStream::connect(&socket).await.is_ok() {
            return Daemon {
                child,
                _directory: directory,
                socket,
            };
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!("gentd exited before creating its local socket: {status}");
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
            "events".into(),
            "host-epoch".into(),
            "receipts".into(),
            ATTACHMENTS_CAPABILITY.into(),
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
        WireFrame::Events { page }
            if page.events.len() == 2
                && page.events[0].kind == "commandAccepted"
                && page.events[1].kind == "commandSettled"
    ));
}

#[tokio::test]
async fn attachment_frames_preserve_transfer_identity_and_resume_durable_progress() {
    let daemon = daemon().await;
    let mut stream = client(&daemon).await;
    let transfer = AttachmentTransfer {
        metadata: AttachmentMetadata {
            attachment_id: "attachment-1".into(),
            display_name: "hello.txt".into(),
            media_type: "text/plain".into(),
            byte_len: 5,
            digest_sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
                .into(),
            storage_key: "sha256/2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
                .into(),
        },
        staging_key: "staging/attachment-1".into(),
        receipt_id: ReceiptId("receipt-1".into()),
        idempotency_key: "attachment-1".into(),
        host_epoch: HostEpoch(1),
        state: AttachmentState::Uploading,
        received_bytes: 0,
    };
    write_json_frame(
        &mut stream,
        &AttachmentFrame::Begin {
            transfer: transfer.clone(),
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        read_json_frame::<_, AttachmentFrame>(&mut stream).await.unwrap(),
        AttachmentFrame::Transfer { transfer: current } if current == transfer
    ));

    let operation = AttachmentOperation {
        attachment_id: "attachment-1".into(),
        transfer_receipt_id: ReceiptId("receipt-1".into()),
        transfer_idempotency_key: "attachment-1".into(),
        receipt_id: ReceiptId("append-receipt-1".into()),
        idempotency_key: "append-attachment-1".into(),
        host_epoch: HostEpoch(1),
    };
    let chunk = AttachmentFrame::Chunk {
        operation: operation.clone(),
        offset: 0,
        data_base64: "aGVsbG8=".into(),
    };
    write_json_frame(&mut stream, &chunk).await.unwrap();
    assert!(matches!(
        read_json_frame::<_, AttachmentFrame>(&mut stream).await.unwrap(),
        AttachmentFrame::Transfer { transfer } if transfer.received_bytes == 5
    ));
    write_json_frame(&mut stream, &chunk).await.unwrap();
    assert!(matches!(
        read_json_frame::<_, AttachmentFrame>(&mut stream).await.unwrap(),
        AttachmentFrame::Transfer { transfer } if transfer.received_bytes == 5
    ));

    write_json_frame(
        &mut stream,
        &AttachmentFrame::Commit {
            operation: AttachmentOperation {
                receipt_id: ReceiptId("commit-receipt-1".into()),
                idempotency_key: "commit-attachment-1".into(),
                ..operation.clone()
            },
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        read_json_frame::<_, AttachmentFrame>(&mut stream).await.unwrap(),
        AttachmentFrame::Transfer { transfer } if transfer.state == AttachmentState::Available
    ));
    write_json_frame(
        &mut stream,
        &AttachmentFrame::Resume {
            attachment_id: "attachment-1".into(),
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        read_json_frame::<_, AttachmentFrame>(&mut stream).await.unwrap(),
        AttachmentFrame::Transfer { transfer } if transfer.state == AttachmentState::Available
    ));
}

#[tokio::test]
async fn observer_daemon_exposes_read_only_doctor_onboarding_and_dependency_plans() {
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
        request(&mut stream, WireFrame::OnboardingRequest).await,
        WireFrame::Onboarding(state) if state.branches.len() == 3
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
