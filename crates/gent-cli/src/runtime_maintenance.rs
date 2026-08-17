//! Protocol-only client for a durable runtime update maintenance report.

use std::path::PathBuf;

use gent_protocol::{
    RUNTIME_MAINTENANCE_CAPABILITY, RuntimeMaintenanceFrame, WireFrame, read_json_frame,
    write_json_frame,
};
use gent_types::{RuntimeMaintenanceReport, RuntimeMaintenanceRequest};
use serde_json::Value;

use crate::local_ipc::connect_and_negotiate;

/// Reads one authority-gated durable update attempt without executing an update effect.
pub(crate) async fn request(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    attempt_id: String,
) -> Result<RuntimeMaintenanceReport, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|capability| capability == RUNTIME_MAINTENANCE_CAPABILITY)
    {
        return Err("gentd does not expose runtime maintenance for this authority profile".into());
    }
    write_json_frame(
        &mut stream,
        &RuntimeMaintenanceFrame::Request(RuntimeMaintenanceRequest { attempt_id }),
    )
    .await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Ok(RuntimeMaintenanceFrame::Report(report)) = serde_json::from_value(raw.clone()) {
        return Ok(*report);
    }
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw) {
        return Err(message.into());
    }
    Err("daemon did not return a runtime maintenance report".into())
}

#[cfg(all(test, unix))]
mod tests {
    use gent_protocol::{Hello, Negotiated, read_frame, write_frame, write_json_frame};
    use gent_types::{
        CapabilitySet, HostEpoch, PROTOCOL_MAX, RuntimeMaintenanceReport,
        RuntimeMaintenanceRequest, RuntimeUpdateHandoff, RuntimeUpdateRecord, RuntimeUpdateStatus,
    };
    use tokio::net::UnixListener;

    use super::{RUNTIME_MAINTENANCE_CAPABILITY, RuntimeMaintenanceFrame, request};

    #[tokio::test]
    async fn request_requires_the_negotiated_maintenance_capability() {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_frame(&mut stream).await.unwrap();
            write_frame(
                &mut stream,
                &gent_protocol::WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet::default(),
                }),
            )
            .await
            .unwrap();
        });
        assert!(
            request(Some(directory.path().into()), true, "attempt-1".into())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn request_decodes_an_exact_attempt_report() {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert!(matches!(
                read_frame(&mut stream).await.unwrap(),
                gent_protocol::WireFrame::Hello(Hello { .. })
            ));
            write_frame(
                &mut stream,
                &gent_protocol::WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet(vec![RUNTIME_MAINTENANCE_CAPABILITY.into()]),
                }),
            )
            .await
            .unwrap();
            assert!(
                matches!(gent_protocol::read_json_frame::<_, RuntimeMaintenanceFrame>(&mut stream).await.unwrap(), RuntimeMaintenanceFrame::Request(RuntimeMaintenanceRequest { attempt_id }) if attempt_id == "attempt-1")
            );
            write_json_frame(
                &mut stream,
                &RuntimeMaintenanceFrame::Report(Box::new(RuntimeMaintenanceReport {
                    host_epoch: HostEpoch(1),
                    ingress_closed: false,
                    record: RuntimeUpdateRecord {
                        attempt_id: "attempt-1".into(),
                        revision: 2,
                        artifact_digest_sha256: "a".repeat(64),
                        status: RuntimeUpdateStatus::default(),
                        handoff: RuntimeUpdateHandoff::default(),
                    },
                })),
            )
            .await
            .unwrap();
        });
        assert_eq!(
            request(Some(directory.path().into()), true, "attempt-1".into())
                .await
                .unwrap()
                .record
                .revision,
            2
        );
    }
}
