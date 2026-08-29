use gent_protocol::{Hello, Negotiated, WireFrame, read_frame, write_frame};
use gent_types::{CapabilitySet, HostEpoch, HostStatus, PROTOCOL_MAX};
use tokio::net::UnixListener;

use super::{
    client_capabilities, connect_and_negotiate, daemon_arguments_from, default_daemon_binary,
    default_data_dir, request, wait_for_connection_until,
};

fn status() -> WireFrame {
    WireFrame::Status(HostStatus {
        host_epoch: HostEpoch(1),
        protocol_min: PROTOCOL_MAX,
        protocol_max: PROTOCOL_MAX,
        capabilities: CapabilitySet(vec!["events".into()]),
    })
}

fn server(directory: &tempfile::TempDir, handshake: WireFrame, response: Option<WireFrame>) {
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        assert!(matches!(
            read_frame(&mut stream).await.unwrap(),
            WireFrame::Hello(Hello { .. })
        ));
        write_frame(&mut stream, &handshake).await.unwrap();
        if let Some(response) = response {
            assert!(matches!(
                read_frame(&mut stream).await.unwrap(),
                WireFrame::StatusRequest
            ));
            write_frame(&mut stream, &response).await.unwrap();
        }
    });
}

fn negotiated() -> WireFrame {
    WireFrame::Negotiated(Negotiated {
        protocol: PROTOCOL_MAX,
        capabilities: CapabilitySet(vec!["events".into()]),
    })
}

#[tokio::test]
async fn request_negotiates_then_returns_the_typed_daemon_response() {
    let directory = tempfile::tempdir().unwrap();
    server(&directory, negotiated(), Some(status()));
    assert!(matches!(
        request(
            Some(directory.path().into()),
            true,
            WireFrame::StatusRequest
        )
        .await,
        Ok(WireFrame::Status(_))
    ));
}

#[tokio::test]
async fn request_rejects_handshake_and_command_errors_without_autostarting() {
    let directory = tempfile::tempdir().unwrap();
    server(
        &directory,
        WireFrame::Error {
            code: "upgradeRequired".into(),
            message: "upgrade".into(),
        },
        None,
    );
    assert!(
        request(
            Some(directory.path().into()),
            true,
            WireFrame::StatusRequest
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("upgrade")
    );
    let missing = tempfile::tempdir().unwrap();
    assert!(
        request(Some(missing.path().into()), true, WireFrame::StatusRequest)
            .await
            .unwrap_err()
            .to_string()
            .contains("--no-autostart")
    );
}

#[tokio::test]
async fn negotiation_is_bounded_when_a_listener_never_replies() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    });
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(4),
        connect_and_negotiate(Some(directory.path().into()), true),
    )
    .await
    .expect("negotiation must be bounded")
    .unwrap_err();
    assert!(error.to_string().contains("did not negotiate"));
}

#[tokio::test]
async fn request_rejects_unexpected_negotiation_and_command_responses() {
    let directory = tempfile::tempdir().unwrap();
    server(&directory, status(), None);
    assert!(
        request(
            Some(directory.path().into()),
            true,
            WireFrame::StatusRequest
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("did not negotiate")
    );
    let directory = tempfile::tempdir().unwrap();
    server(
        &directory,
        negotiated(),
        Some(WireFrame::Error {
            code: "invalidCommand".into(),
            message: "denied".into(),
        }),
    );
    assert!(
        request(
            Some(directory.path().into()),
            true,
            WireFrame::StatusRequest
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("denied")
    );
}

#[test]
fn defaults_resolve_to_non_empty_local_paths() {
    assert!(default_daemon_binary().file_name().is_some());
    assert_eq!(
        default_data_dir().file_name(),
        Some(std::ffi::OsStr::new(".gentd"))
    );
}

#[test]
fn client_requests_the_exact_turn_follow_capability() {
    assert!(
        client_capabilities()
            .0
            .iter()
            .any(|capability| capability == gent_protocol::AGENT_CHAT_TURN_FOLLOW_CAPABILITY)
    );
}

#[test]
fn client_requests_daemon_owned_provider_readiness_and_provisioning() {
    let capabilities = client_capabilities();
    assert!(
        capabilities
            .0
            .contains(&gent_protocol::PROVIDER_READINESS_CAPABILITY.into())
    );
    assert!(
        capabilities
            .0
            .contains(&gent_protocol::PROMPT_PROVIDER_PROVISION_CAPABILITY.into())
    );
}

#[test]
fn client_requests_private_conversation_content_on_every_platform() {
    assert!(
        client_capabilities()
            .0
            .contains(&gent_protocol::CONVERSATION_CONTENT_CAPABILITY.into())
    );
}

#[test]
fn client_requests_workspace_git() {
    assert!(
        client_capabilities()
            .0
            .contains(&gent_protocol::WORKSPACE_GIT_CAPABILITY.into())
    );
}

#[test]
fn daemon_arguments_always_select_standalone_authority() {
    let directory = tempfile::tempdir().unwrap();
    let arguments = daemon_arguments_from(directory.path());
    assert_eq!(arguments[0], "--data-dir");
    assert_eq!(arguments[2], "--standalone-authority");
    assert_eq!(arguments.len(), 3);
}

#[tokio::test]
async fn wait_for_connection_retries_until_a_listener_is_ready() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("gentd.sock");
    let listener = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let _listener = UnixListener::bind(socket).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(75)).await;
    });
    assert!(
        wait_for_connection_until(directory.path(), None)
            .await
            .is_ok()
    );
    listener.await.unwrap();
}

#[tokio::test]
async fn spawned_daemon_failure_is_reported_without_waiting_for_the_timeout() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = tokio::process::Command::new("sh")
        .args(["-c", "exit 17"])
        .spawn()
        .unwrap();
    let error = super::wait_for_spawned_connection(directory.path(), &mut child)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("exited before becoming ready"));
}
