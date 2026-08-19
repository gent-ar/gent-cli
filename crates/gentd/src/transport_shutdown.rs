//! Cancellation-aware local IPC serving, separated from protocol dispatch and daemon authority.

#[cfg(unix)]
use std::io::Error;
#[cfg(unix)]
use std::time::Duration;

use tokio::sync::watch;
#[cfg(unix)]
use tokio::task::JoinSet;

#[cfg(unix)]
use tokio::net::UnixListener;

use crate::api::RuntimeApi;

#[cfg(unix)]
const CONNECTION_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

/// Process-local stop signal shared by the listener and every accepted IPC connection.
///
/// This is transient control state, not a recovery snapshot or a durable lifecycle fact.
#[derive(Clone, Debug)]
pub(crate) struct TransportShutdown {
    sender: watch::Sender<bool>,
}

impl TransportShutdown {
    /// Creates an open signal. Calling [`Self::request`] is idempotent.
    #[must_use]
    pub(crate) fn new() -> Self {
        let (sender, _) = watch::channel(false);
        Self { sender }
    }

    /// Stops new accepts and cancels all current protocol-serving futures.
    pub(crate) fn request(&self) {
        self.sender.send_replace(true);
    }

    /// Waits until shutdown was requested, including a request that preceded this call.
    pub(crate) async fn cancelled(&self) {
        let mut receiver = self.sender.subscribe();
        if *receiver.borrow() {
            return;
        }
        let _ = receiver.changed().await;
    }
}

#[cfg(unix)]
/// Serves local IPC until requested to stop, then waits a bounded time for connections to drain.
pub(crate) async fn serve_until<R: RuntimeApi>(
    listener: UnixListener,
    runtime: R,
    shutdown: TransportShutdown,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let runtime = runtime.clone();
                let shutdown = shutdown.clone();
                connections.spawn(async move {
                    if let Err(error) = crate::transport::serve_connection_until(stream, runtime, shutdown).await {
                        eprintln!("gentd connection closed: {error}");
                    }
                });
            }
            joined = connections.join_next(), if !connections.is_empty() => {
                if let Some(Err(error)) = joined {
                    eprintln!("gentd connection task ended unexpectedly: {error}");
                }
            }
        }
    }
    drain_connections(&mut connections).await
}

#[cfg(unix)]
async fn drain_connections(
    connections: &mut JoinSet<()>,
) -> Result<(), Box<dyn std::error::Error>> {
    let drained = tokio::time::timeout(CONNECTION_DRAIN_TIMEOUT, async {
        while connections.join_next().await.is_some() {}
    })
    .await;
    if drained.is_ok() {
        return Ok(());
    }
    eprintln!(
        "gentd IPC drain exceeded the shutdown deadline; waiting for graceful connection closure"
    );
    while connections.join_next().await.is_some() {}
    Err(Box::new(Error::other(
        "gentd IPC connections did not stop before the shutdown deadline",
    )))
}
