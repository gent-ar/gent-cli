//! Decision transport boundaries: recovery is client-controlled, provider facts are not.

use gent_protocol::{
    DecisionEvidence, DecisionRecoveryEvidence, DependencyProvider, PublicRunOutcome,
    PublicRunStartRequest, WireFrame, read_frame, write_frame,
};
use gent_types::HostEpoch;
use tokio::io::duplex;

use crate::transport::serve_connection;
use crate::transport_tests::{FakeRuntime, hello};

#[tokio::test]
async fn decision_recovery_and_provider_lifecycle_are_routed_after_handshake() {
    let (mut client, server) = duplex(1024);
    let task = tokio::spawn(serve_connection(server, FakeRuntime));
    write_frame(&mut client, &hello()).await.unwrap();
    let _ = read_frame(&mut client).await.unwrap();
    write_frame(
        &mut client,
        &WireFrame::DecisionRecovery {
            decision_id: "decision-1".into(),
            evidence: DecisionRecoveryEvidence::AcknowledgementUnprovable,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        read_frame(&mut client).await.unwrap(),
        WireFrame::DecisionSettlement(decision) if decision.phase.is_terminal()
    ));
    write_frame(
        &mut client,
        &WireFrame::DecisionEvidence {
            decision_id: "decision-1".into(),
            evidence: DecisionEvidence::ProviderSettled,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        read_frame(&mut client).await.unwrap(),
        WireFrame::Error { code, message }
            if code == "invalidCommand" && message.contains("daemon-owned lifecycle")
    ));
    let run = PublicRunStartRequest {
        run_id: "run".into(),
        coordinator_id: "host".into(),
        host_epoch: HostEpoch(1),
        provider: DependencyProvider::Claude,
        executable: "/tmp/claude".into(),
        version: "1".into(),
        compatibility_entry: "fixture".into(),
    };
    write_frame(&mut client, &WireFrame::PublicRunStart(run))
        .await
        .unwrap();
    assert!(matches!(
        read_frame(&mut client).await.unwrap(),
        WireFrame::PublicRunResponse(response) if response.outcome == PublicRunOutcome::Denied
    ));
    drop(client);
    assert!(task.await.unwrap().is_err());
}
