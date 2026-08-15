//! Windows named-pipe listener; protocol handling stays transport-neutral.

use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

use crate::api::RuntimeApi;
use crate::transport::serve_connection;

/// Serves local named-pipe clients with one listener instance per pending client.
pub(crate) async fn serve_named_pipe<R: RuntimeApi>(
    pipe_name: &str,
    runtime: R,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut first = true;
    let mut server = create_server(pipe_name, first)?;
    first = false;
    loop {
        server.connect().await?;
        // Keep a listening instance available before the connected one is handed
        // to its task. Windows otherwise reports a transient busy/not-found pipe
        // to the next short-lived CLI request.
        let replacement = create_server(pipe_name, first)?;
        let runtime = runtime.clone();
        tokio::spawn(async move {
            if let Err(error) = serve_connection(server, runtime).await {
                eprintln!("gentd connection closed: {error}");
            }
        });
        server = replacement;
    }
}

fn create_server(pipe_name: &str, first: bool) -> std::io::Result<NamedPipeServer> {
    ServerOptions::new()
        .first_pipe_instance(first)
        .create(pipe_name)
}
