//! Windows transport checks that need native named-pipe support.

use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};

use crate::transport::tests::FakeRuntime;
use crate::transport_windows::serve_named_pipe;

#[tokio::test]
async fn named_pipe_accepts_a_local_client() {
    let name = format!(r"\\.\pipe\gentd-test-{}", std::process::id());
    let server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&name)
        .unwrap();
    let client = ClientOptions::new().open(&name).unwrap();
    server.connect().await.unwrap();
    drop(client);
}

#[tokio::test]
async fn replacement_instance_accepts_the_next_client() {
    let name = format!(r"\\.\pipe\gentd-replacement-{}", std::process::id());
    let first = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&name)
        .unwrap();
    let first_client = ClientOptions::new().open(&name).unwrap();
    first.connect().await.unwrap();

    // A listener must reserve this instance before dispatching the first client.
    let replacement = ServerOptions::new().create(&name).unwrap();
    let second_client = ClientOptions::new().open(&name).unwrap();
    replacement.connect().await.unwrap();

    drop((first_client, second_client));
}

/// `serve_named_pipe` always has *some* instance-count limit at any single
/// instant — a Windows named pipe fails a connect immediately with
/// ERROR_PIPE_BUSY rather than queuing it the way a TCP listen() backlog
/// would, so no server-side accept-loop timing alone can promise a truly
/// simultaneous burst never sees it. The real guarantee gentd offers is the
/// one every named-pipe client is expected to honor (and what
/// `gentd_ipc.rs::connect_local` on the Mate app side now does): retry
/// briefly on ERROR_PIPE_BUSY. This is the regression case for the accept
/// loop's own contribution — closing the gap between a `connect().await`
/// returning and its replacement instance existing, which is wide enough in
/// practice (a mux-relayed message arriving while an unrelated gentd
/// connection was also opening) that even a *retrying* client only gets one
/// or two tries in before its short first retry delay — a burst of clients
/// that each retry past ERROR_PIPE_BUSY must all still connect promptly.
#[tokio::test]
async fn a_burst_of_retrying_clients_all_connect_promptly() {
    let name = format!(r"\\.\pipe\gentd-burst-{}", std::process::id());
    let pipe_name = name.clone();
    tokio::spawn(async move {
        let _ = serve_named_pipe(&pipe_name, FakeRuntime).await;
    });

    // Give the listener a moment to create its first instance.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let attempts = 32;
    let mut connects = tokio::task::JoinSet::new();
    for _ in 0..attempts {
        let name = name.clone();
        connects.spawn_blocking(move || connect_with_retry(&name));
    }

    let mut succeeded = 0;
    while let Some(result) = connects.join_next().await {
        result.unwrap().expect("a retrying client must eventually connect");
        succeeded += 1;
    }

    assert_eq!(succeeded, attempts);
}

/// Mirrors the retry `gentd_ipc.rs::connect_local` performs on the Mate app
/// side: ERROR_PIPE_BUSY is expected, ordinary concurrency, not a failure.
fn connect_with_retry(name: &str) -> std::io::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match ClientOptions::new().open(name) {
            Ok(client) => return Ok(drop(client)),
            Err(error) if error.raw_os_error() == Some(231) && std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(error) => return Err(error),
        }
    }
}
