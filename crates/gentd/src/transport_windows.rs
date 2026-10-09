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
        // Create the next listening instance BEFORE awaiting a connection on
        // this one, not after. Windows named pipes fail a concurrent connect
        // attempt immediately with ERROR_PIPE_BUSY rather than queuing it, so
        // the moment between `connect().await` returning and a replacement
        // instance existing is a real window with zero listeners — any client
        // that arrives in it sees "all pipe instances are busy" even though
        // gentd is healthy. Pre-creating the next instance keeps at least one
        // always live, closing that window.
        let next = create_server(pipe_name, first)?;
        server.connect().await?;
        let runtime = runtime.clone();
        tokio::spawn(async move {
            if let Err(error) = serve_connection(server, runtime).await {
                eprintln!("gentd connection closed: {error}");
            }
        });
        server = next;
    }
}

fn create_server(pipe_name: &str, first: bool) -> std::io::Result<NamedPipeServer> {
    ServerOptions::new()
        .first_pipe_instance(first)
        .create(pipe_name)
}
