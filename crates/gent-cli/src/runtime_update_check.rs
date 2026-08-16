//! Protocol-only client for the daemon's optional cached update report.

use std::path::PathBuf;

use gent_protocol::{
    RUNTIME_UPDATE_CHECK_CAPABILITY, RuntimeUpdateCheckFrame, WireFrame, read_json_frame,
    write_json_frame,
};
use gent_types::{RuntimeReleaseChannel, RuntimeUpdateCheckReport, RuntimeUpdateCheckRequest};
use serde_json::Value;

use crate::local_ipc::connect_and_negotiate;

/// Reads an explicitly configured daemon's signed cached release report.
///
/// # Errors
/// Returns an error when the daemon did not negotiate the report-only capability.
pub(crate) async fn request(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    channel: RuntimeReleaseChannel,
) -> Result<RuntimeUpdateCheckReport, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|capability| capability == RUNTIME_UPDATE_CHECK_CAPABILITY)
    {
        return Err(
            "gentd does not expose signed cached update checks; start it with --runtime-update-check-authority and trusted cache configuration"
                .into(),
        );
    }
    write_json_frame(
        &mut stream,
        &RuntimeUpdateCheckFrame::Request(RuntimeUpdateCheckRequest { channel }),
    )
    .await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Ok(RuntimeUpdateCheckFrame::Report(report)) = serde_json::from_value(raw.clone()) {
        return Ok(report);
    }
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw) {
        return Err(message.into());
    }
    Err("daemon did not return a runtime update report".into())
}

#[cfg(all(test, unix))]
mod tests {
    use gent_protocol::{Hello, Negotiated, read_frame, write_frame, write_json_frame};
    use gent_types::{
        CapabilitySet, PROTOCOL_MAX, RuntimeReleaseChannel, RuntimeUpdateCheckReport,
        RuntimeUpdateCheckRequest, RuntimeUpdateCheckState, RuntimeVersion,
    };
    use tokio::net::UnixListener;

    use super::{RUNTIME_UPDATE_CHECK_CAPABILITY, RuntimeUpdateCheckFrame, WireFrame, request};

    #[tokio::test]
    async fn request_requires_the_negotiated_update_capability() {
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
        let error = request(
            Some(directory.path().into()),
            true,
            RuntimeReleaseChannel::Stable,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("does not expose"));
    }

    #[tokio::test]
    async fn request_decodes_a_signed_daemon_report() {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert!(matches!(
                read_frame(&mut stream).await.unwrap(),
                WireFrame::Hello(Hello { .. })
            ));
            write_frame(
                &mut stream,
                &WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet(vec![RUNTIME_UPDATE_CHECK_CAPABILITY.into()]),
                }),
            )
            .await
            .unwrap();
            assert!(matches!(
                gent_protocol::read_json_frame::<_, RuntimeUpdateCheckFrame>(&mut stream)
                    .await
                    .unwrap(),
                RuntimeUpdateCheckFrame::Request(RuntimeUpdateCheckRequest {
                    channel: RuntimeReleaseChannel::Stable
                })
            ));
            write_json_frame(
                &mut stream,
                &RuntimeUpdateCheckFrame::Report(RuntimeUpdateCheckReport {
                    current_version: RuntimeVersion {
                        major: 1,
                        minor: 0,
                        patch: 0,
                    },
                    channel: RuntimeReleaseChannel::Stable,
                    state: RuntimeUpdateCheckState::Current,
                    candidate: None,
                    failure: None,
                }),
            )
            .await
            .unwrap();
        });
        assert_eq!(
            request(
                Some(directory.path().into()),
                true,
                RuntimeReleaseChannel::Stable
            )
            .await
            .unwrap()
            .state,
            RuntimeUpdateCheckState::Current
        );
    }
}
