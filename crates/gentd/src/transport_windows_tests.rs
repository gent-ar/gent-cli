//! Windows transport checks that need native named-pipe support.

use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};

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
