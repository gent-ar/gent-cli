//! Windows named-pipe listener; protocol handling stays transport-neutral.

use tokio::net::windows::named_pipe::ServerOptions;

use crate::transport::{RuntimeApi, serve_connection};

/// Serves local named-pipe clients with one listener instance per pending client.
pub(crate) async fn serve_named_pipe<R: RuntimeApi>(
    pipe_name: &str,
    runtime: R,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut first = true;
    loop {
        let server = ServerOptions::new()
            .first_pipe_instance(first)
            .create(pipe_name)?;
        first = false;
        server.connect().await?;
        let runtime = runtime.clone();
        tokio::spawn(async move {
            if let Err(error) = serve_connection(server, runtime).await {
                eprintln!("gentd connection closed: {error}");
            }
        });
    }
}
