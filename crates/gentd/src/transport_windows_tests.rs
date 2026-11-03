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

#[tokio::test]
async fn a_burst_of_retrying_clients_all_connect_promptly() {
    let name = format!(r"\\.\pipe\gentd-burst-{}", std::process::id());
    let pipe_name = name.clone();
    tokio::spawn(async move {
        let _ = serve_named_pipe(&pipe_name, FakeRuntime).await;
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let attempts = 32;
    let mut connects = tokio::task::JoinSet::new();
    for _ in 0..attempts {
        let name = name.clone();
        connects.spawn_blocking(move || connect_with_retry(&name));
    }

    let mut succeeded = 0;
    while let Some(result) = connects.join_next().await {
        result
            .unwrap()
            .expect("a retrying client must eventually connect");
        succeeded += 1;
    }

    assert_eq!(succeeded, attempts);
}

fn connect_with_retry(name: &str) -> std::io::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match ClientOptions::new().open(name) {
            Ok(client) => {
                return {
                    let _: () = drop(client);
                    Ok(())
                };
            }
            Err(error)
                if error.raw_os_error() == Some(231) && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(error) => return Err(error),
        }
    }
}
