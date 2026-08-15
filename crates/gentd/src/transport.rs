//! Local IPC adapter. It only knows the `RuntimeApi` port, never persistence or providers.

use gent_protocol::{WireFrame, negotiate, read_frame, write_frame};
use gent_types::{CapabilitySet, Command, Event, HostStatus, PROTOCOL_MAX, PROTOCOL_MIN, Receipt};
use tokio::net::{UnixListener, UnixStream};

const CAPABILITIES: &[&str] = &["events", "host-epoch", "receipts"];

pub trait RuntimeApi: Clone + Send + Sync + 'static {
    fn status(&self) -> Result<HostStatus, String>;
    fn submit(&self, command: Command) -> Result<Receipt, String>;
    fn events_after(&self, cursor: u64) -> Result<Vec<Event>, String>;
}

pub async fn serve<R: RuntimeApi>(
    listener: UnixListener,
    runtime: R,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        let (stream, _) = listener.accept().await?;
        let runtime = runtime.clone();
        tokio::spawn(async move {
            if let Err(error) = serve_connection(stream, runtime).await {
                eprintln!("gentd connection closed: {error}");
            }
        });
    }
}

async fn serve_connection<R: RuntimeApi>(
    mut stream: UnixStream,
    runtime: R,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let WireFrame::Hello(hello) = read_frame(&mut stream).await? else {
        return write_error(
            &mut stream,
            "handshakeRequired",
            "hello must be the first frame",
        )
        .await;
    };
    let capabilities = CapabilitySet(CAPABILITIES.iter().map(ToString::to_string).collect());
    match negotiate(&hello, PROTOCOL_MIN, PROTOCOL_MAX, &capabilities) {
        Ok(answer) => write_frame(&mut stream, &WireFrame::Negotiated(answer)).await?,
        Err(error) => return write_error(&mut stream, "upgradeRequired", &error.to_string()).await,
    }
    loop {
        let frame = match read_frame(&mut stream).await? {
            WireFrame::StatusRequest => runtime.status().map(WireFrame::Status),
            WireFrame::Command(command) => runtime.submit(command).map(WireFrame::Receipt),
            WireFrame::Subscribe { after_cursor } => runtime
                .events_after(after_cursor)
                .map(|events| WireFrame::Events { events }),
            _ => Err("frame is not valid after negotiation".into()),
        };
        match frame {
            Ok(frame) => write_frame(&mut stream, &frame).await?,
            Err(message) => write_error(&mut stream, "invalidCommand", &message).await?,
        }
    }
}

async fn write_error(
    stream: &mut UnixStream,
    code: &str,
    message: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    write_frame(
        stream,
        &WireFrame::Error {
            code: code.into(),
            message: message.into(),
        },
    )
    .await?;
    Ok(())
}
