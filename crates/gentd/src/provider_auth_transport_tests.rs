//! Observer-safety tests for the reserved provider-auth daemon boundary.

use gent_protocol::{
    PROVIDER_AUTH_CAPABILITY, ProviderAuthFrame, WireFrame, read_frame, write_frame,
    write_json_frame,
};
use gent_runtime::catalog::RuntimeCapabilityProfile;
use gent_types::ProviderAuthProvider;
use tokio::io::duplex;

use crate::{
    CompatibilityAssessment,
    api::RuntimeApi,
    build_runtime,
    transport::serve_connection,
    transport_tests::{FakeRuntime, hello},
};

#[test]
fn observer_neither_advertises_nor_starts_provider_authentication() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = build_runtime(
        directory.path(),
        &RuntimeCapabilityProfile::default(),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    assert!(
        !runtime
            .capabilities()
            .unwrap()
            .0
            .iter()
            .any(|capability| capability == PROVIDER_AUTH_CAPABILITY)
    );

    let error = runtime
        .provider_auth(ProviderAuthFrame::LoginRequest {
            request_id: "request-1".into(),
            provider: ProviderAuthProvider::Claude,
        })
        .unwrap_err();
    assert_eq!(
        error,
        "provider authentication is unavailable while gentd is observer-disabled"
    );
}

#[tokio::test]
async fn observer_rejects_provider_auth_before_any_login_handler_can_run() {
    let (mut client, server) = duplex(1024);
    let task = tokio::spawn(serve_connection(server, FakeRuntime));
    write_frame(&mut client, &hello()).await.unwrap();
    assert!(matches!(
        read_frame(&mut client).await.unwrap(),
        WireFrame::Negotiated(answer)
            if !answer.capabilities.0.contains(&PROVIDER_AUTH_CAPABILITY.into())
    ));
    write_json_frame(
        &mut client,
        &ProviderAuthFrame::LoginRequest {
            request_id: "request-1".into(),
            provider: ProviderAuthProvider::Codex,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        read_frame(&mut client).await.unwrap(),
        WireFrame::Error { code, .. } if code == "invalidCommand"
    ));
    drop(client);
    assert!(task.await.unwrap().is_err());
}
