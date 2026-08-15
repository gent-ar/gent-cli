#![cfg(unix)]

use std::process::Command;
use std::sync::{Arc, Mutex};

use gent_protocol::{Hello, Negotiated, WireFrame, read_frame, write_frame};
use gent_types::{CapabilitySet, HostEpoch, HostStatus, PROTOCOL_MAX};
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
            saved.lock().unwrap().push(request);
            write_frame(&mut stream, &status()).await.unwrap();
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("hostEpoch"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_maps_every_public_command_to_a_negotiated_protocol_frame() {
    let directory = tempfile::tempdir().unwrap();
    let received = server(&directory, 13);
    for args in [
        &["doctor"][..],
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
        &["decision", "ack", "--decision-id", "d1"],
        &["decision", "settle", "--decision-id", "d1"],
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
    assert!(matches!(received[1], WireFrame::DependencyPlanRequest(_)));
    assert!(
        matches!(received[2], WireFrame::DependencyActionRequest(ref request) if !request.consent_granted)
    );
    assert!(
        matches!(received[3], WireFrame::DependencyActionRequest(ref request) if request.consent_granted)
    );
    assert!(matches!(received[4], WireFrame::StatusRequest));
    assert!(matches!(
        received[5],
        WireFrame::Subscribe { after_cursor: 2 }
    ));
    assert!(matches!(received[6], WireFrame::DecisionSubmit(_)));
    for frame in &received[7..11] {
        assert!(matches!(frame, WireFrame::DecisionEvidence { .. }));
    }
    assert!(matches!(received[11], WireFrame::StatusRequest));
    assert!(matches!(
        received[12],
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
