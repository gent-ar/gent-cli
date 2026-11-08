use std::time::Duration;

use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::task::JoinSet;

use crate::api::RuntimeApi;
use crate::transport::serve_connection;

pub(crate) const LISTENING_INSTANCES: usize = 4;
const INSTANCE_RETRY: Duration = Duration::from_millis(100);

pub(crate) async fn serve_named_pipe<R: RuntimeApi>(
    pipe_name: &str,
    runtime: R,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut first = Some(
        ServerOptions::new()
            .first_pipe_instance(true)
            .create(pipe_name)?,
    );
    let mut acceptors = JoinSet::new();
    for _ in 0..LISTENING_INSTANCES {
        acceptors.spawn(accept(pipe_name.to_owned(), first.take(), runtime.clone()));
    }
    match acceptors.join_next().await {
        Some(Err(error)) => Err(error.into()),
        _ => Err("named-pipe acceptors stopped".into()),
    }
}

async fn accept<R: RuntimeApi>(
    pipe_name: String,
    mut reserved: Option<NamedPipeServer>,
    runtime: R,
) {
    loop {
        let server = match reserved
            .take()
            .map_or_else(|| ServerOptions::new().create(&pipe_name), Ok)
        {
            Ok(server) => server,
            Err(error) => {
                eprintln!("gentd could not open a named-pipe instance: {error}");
                tokio::time::sleep(INSTANCE_RETRY).await;
                continue;
            }
        };
        if let Err(error) = server.connect().await {
            eprintln!("gentd named-pipe client left before it was served: {error}");
            continue;
        }
        let runtime = runtime.clone();
        tokio::spawn(async move {
            if let Err(error) = serve_connection(server, runtime).await {
                eprintln!("gentd connection closed: {error}");
            }
        });
    }
}
