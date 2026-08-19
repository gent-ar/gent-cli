use std::time::Duration;

use gent_protocol::{read_frame, write_frame};
use tokio::io::AsyncReadExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::time::timeout;

use crate::transport_shutdown::{TransportShutdown, serve_until};
use crate::transport_tests::FakeRuntime;
use crate::transport_tests::hello;

#[tokio::test]
async fn shutdown_stops_listener_and_closes_a_pending_connection() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("gentd.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let shutdown = TransportShutdown::new();
    let server_shutdown = shutdown.clone();
    let server = tokio::spawn(async move {
        serve_until(listener, FakeRuntime, server_shutdown)
            .await
            .unwrap();
    });
    let mut client = UnixStream::connect(&socket).await.unwrap();
    write_frame(&mut client, &hello()).await.unwrap();
    let _ = read_frame(&mut client).await.unwrap();

    shutdown.request();

    timeout(Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap();
    let mut byte = [0_u8; 1];
    assert_eq!(
        timeout(Duration::from_secs(1), client.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert!(UnixStream::connect(&socket).await.is_err());
}

#[tokio::test]
async fn shutdown_requested_before_serving_returns_without_accepting() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("gentd.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let shutdown = TransportShutdown::new();
    shutdown.request();

    timeout(
        Duration::from_secs(1),
        serve_until(listener, FakeRuntime, shutdown),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(UnixStream::connect(&socket).await.is_err());
}
