//! Bidirectional newline-delimited JSON-RPC 2.0 connection over arbitrary
//! `AsyncRead` / `AsyncWrite` half-duplex pair (typically stdin/stdout).
//!
//! Both sides can send requests, responses, and notifications. Outbound
//! requests are tracked in a pending map so their responses can be routed
//! back to the awaiting caller.
//!
//! The wire format matches the Agent Client Protocol: each message is a
//! single UTF-8 line terminated by `\n` carrying a JSON-RPC 2.0 envelope.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use agent_client_protocol_schema as acp;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{debug, error, trace, warn};

const JSONRPC_VERSION: &str = "2.0";

/// One parsed inbound JSON-RPC message — request, notification, or response.
///
/// Responses are NOT included here because the connection routes them
/// internally to the awaiting `send_request` future.
#[derive(Debug)]
pub enum Inbound {
    Request {
        id: acp::RequestId,
        method: String,
        params: Option<Value>,
    },
    Notification {
        method: String,
        params: Option<Value>,
    },
}

/// Bidirectional connection over newline-delimited JSON-RPC.
///
/// Cloneable senders are obtained via `Connection::sender()`. The receive
/// loop is driven by `Connection::run_reader()` which yields parsed inbound
/// requests / notifications to the caller via the `mpsc::Receiver` returned
/// by `Connection::new`.
pub struct Connection {
    writer: Mutex<Box<dyn AsyncWrite + Send + Unpin>>,
    pending: parking_lot::Mutex<PendingRequests>,
    pub(crate) closed_token: tokio_util::sync::CancellationToken,
    next_outbound_id: AtomicI64,
}

struct PendingRequests {
    closed: bool,
    requests: HashMap<String, oneshot::Sender<Result<Value, acp::Error>>>,
}

impl Connection {
    /// Create a new connection over the given writer. The caller drives the
    /// reader by calling `run_reader` separately.
    pub fn new<W>(writer: W) -> Arc<Self>
    where
        W: AsyncWrite + Send + Unpin + 'static,
    {
        Arc::new(Self {
            writer: Mutex::new(Box::new(writer)),
            pending: parking_lot::Mutex::new(PendingRequests {
                closed: false,
                requests: HashMap::new(),
            }),
            closed_token: tokio_util::sync::CancellationToken::new(),
            next_outbound_id: AtomicI64::new(1),
        })
    }

    /// Send a successful JSON-RPC response to a previously received request.
    pub async fn send_response<R: Serialize>(
        self: &Arc<Self>,
        id: acp::RequestId,
        result: R,
    ) -> anyhow::Result<()> {
        let value = serde_json::to_value(result)?;
        let msg = serde_json::json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": id,
            "result": value,
        });
        self.write_line(&msg).await
    }

    /// Send a JSON-RPC error response.
    pub async fn send_error_response(
        self: &Arc<Self>,
        id: acp::RequestId,
        error: acp::Error,
    ) -> anyhow::Result<()> {
        let msg = serde_json::json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": id,
            "error": error,
        });
        self.write_line(&msg).await
    }

    /// Send a notification (no id, no response expected).
    pub async fn send_notification<P: Serialize>(
        self: &Arc<Self>,
        method: &str,
        params: P,
    ) -> anyhow::Result<()> {
        let value = serde_json::to_value(params)?;
        let msg = serde_json::json!({
            "jsonrpc": JSONRPC_VERSION,
            "method": method,
            "params": value,
        });
        self.write_line(&msg).await
    }

    /// Send an outbound request and await the response.
    ///
    /// The caller is blocked until the matching response arrives via the
    /// reader loop, or the connection is closed (in which case `Err(...)`).
    pub async fn send_request<P: Serialize, R: DeserializeOwned>(
        self: &Arc<Self>,
        method: &str,
        params: P,
    ) -> anyhow::Result<Result<R, acp::Error>> {
        let raw_id = self.next_outbound_id.fetch_add(1, Ordering::Relaxed);
        let id_key = format!("n:{}", raw_id);
        let (tx, rx) = oneshot::channel();
        let params_value = serde_json::to_value(params)?;
        {
            let mut pending = self.pending.lock();
            if pending.closed {
                return Err(anyhow::anyhow!("connection closed"));
            }
            pending.requests.insert(id_key.clone(), tx);
        }
        let msg = serde_json::json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": raw_id,
            "method": method,
            "params": params_value,
        });
        if let Err(e) = self.write_line(&msg).await {
            self.pending.lock().requests.remove(&id_key);
            return Err(e);
        }

        match rx.await {
            Ok(Ok(value)) => {
                let typed: R = serde_json::from_value(value)?;
                Ok(Ok(typed))
            }
            Ok(Err(err)) => Ok(Err(err)),
            Err(_) => Err(anyhow::anyhow!(
                "connection closed while awaiting response to '{}'",
                method
            )),
        }
    }

    async fn write_line(self: &Arc<Self>, value: &Value) -> anyhow::Result<()> {
        let mut buf = serde_json::to_vec(value)?;
        buf.push(b'\n');
        let mut writer = tokio::select! {
            biased;
            _ = self.closed_token.cancelled() => return Err(anyhow::anyhow!("connection closed before write")),
            writer = self.writer.lock() => writer,
        };
        let mut offset = 0;
        while offset < buf.len() {
            let written = tokio::select! {
                biased;
                _ = self.closed_token.cancelled() => return Err(anyhow::anyhow!("connection closed during write")),
                result = writer.write(&buf[offset..]) => match result {
                    Ok(written) => written,
                    Err(error) => { self.close(); return Err(error.into()); }
                },
            };
            if written == 0 {
                self.close();
                return Err(anyhow::anyhow!("connection writer closed"));
            }
            offset += written;
        }
        tokio::select! {
            biased;
            _ = self.closed_token.cancelled() => return Err(anyhow::anyhow!("connection closed during flush")),
            result = writer.flush() => if let Err(error) = result { self.close(); return Err(error.into()); },
        }
        trace!(bytes = buf.len(), "ACP wire send");
        Ok(())
    }

    /// Look up and resolve a pending outbound request when a response arrives.
    fn complete_pending(&self, id: &acp::RequestId, payload: Result<Value, acp::Error>) {
        let key = id_to_key(id);
        if let Some(tx) = self.pending.lock().requests.remove(&key) {
            let _ = tx.send(payload);
        } else {
            warn!(?id, "ACP: received response for unknown request id");
        }
    }

    fn close(&self) {
        let mut pending = self.pending.lock();
        pending.closed = true;
        pending.requests.clear();
        drop(pending);
        self.closed_token.cancel();
    }
}

fn id_to_key(id: &acp::RequestId) -> String {
    match id {
        acp::RequestId::Null => "null".to_string(),
        acp::RequestId::Number(n) => format!("n:{}", n),
        acp::RequestId::Str(s) => format!("s:{}", s),
    }
}

struct OutputCommand {
    bytes: Vec<u8>,
    completed: oneshot::Sender<std::io::Result<()>>,
}

struct ThreadedOutput {
    sender: tokio_util::sync::PollSender<OutputCommand>,
    pending: Option<oneshot::Receiver<std::io::Result<()>>>,
}

impl ThreadedOutput {
    fn poll_pending(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        use std::future::Future;
        use std::task::Poll;
        let Some(pending) = self.pending.as_mut() else {
            return Poll::Ready(Ok(()));
        };
        match std::pin::Pin::new(pending).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => {
                self.pending = None;
                Poll::Ready(result.unwrap_or_else(|_| {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "stdout owner stopped",
                    ))
                }))
            }
        }
    }
}

impl tokio::io::AsyncWrite for ThreadedOutput {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        use std::task::Poll;
        std::task::ready!(self.poll_pending(cx))?;
        if bytes.is_empty() {
            return Poll::Ready(Ok(0));
        }
        std::task::ready!(self.sender.poll_reserve(cx)).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "stdout owner closed")
        })?;
        let length = bytes.len().min(8192);
        let (completed, pending) = oneshot::channel();
        self.sender
            .send_item(OutputCommand {
                bytes: bytes[..length].to_vec(),
                completed,
            })
            .map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "stdout owner closed")
            })?;
        self.pending = Some(pending);
        Poll::Ready(Ok(length))
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.poll_pending(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::ready!(self.poll_pending(cx))?;
        self.sender.close();
        std::task::Poll::Ready(Ok(()))
    }
}

pub(crate) fn threaded_output<W>(
    mut writer: W,
) -> (
    impl tokio::io::AsyncWrite + Unpin + Send,
    std::thread::JoinHandle<()>,
)
where
    W: std::io::Write + Send + 'static,
{
    let (sender, mut receiver) = mpsc::channel::<OutputCommand>(1);
    let thread = std::thread::spawn(move || {
        while let Some(command) = receiver.blocking_recv() {
            let result = writer
                .write_all(&command.bytes)
                .and_then(|_| writer.flush());
            let failed = result.is_err();
            let _ = command.completed.send(result);
            if failed {
                break;
            }
        }
    });
    (
        ThreadedOutput {
            sender: tokio_util::sync::PollSender::new(sender),
            pending: None,
        },
        thread,
    )
}

pub(crate) fn threaded_input<R>(
    mut reader: R,
) -> (
    impl tokio::io::AsyncRead + Unpin,
    std::thread::JoinHandle<()>,
)
where
    R: std::io::Read + Send + 'static,
{
    use futures::StreamExt;
    let (sender, receiver) = mpsc::channel::<std::io::Result<bytes::Bytes>>(1);
    let thread = std::thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    if sender
                        .blocking_send(Ok(bytes::Bytes::copy_from_slice(&buffer[..n])))
                        .is_err()
                    {
                        break;
                    }
                }
                Err(error) => {
                    let _ = sender.blocking_send(Err(error));
                    break;
                }
            }
        }
    });
    let stream = futures::stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|item| (item, receiver))
    })
    .boxed();
    (tokio_util::io::StreamReader::new(stream), thread)
}

/// Spawn the reader loop. Inbound requests and notifications are forwarded
/// to `tx`. Responses are matched against `connection.pending` internally.
/// The future completes when stdin reaches EOF.
pub async fn run_reader<R>(
    connection: Arc<Connection>,
    reader: R,
    tx: mpsc::UnboundedSender<Inbound>,
) -> anyhow::Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buffered = BufReader::new(reader);
    let mut line = Vec::new();
    let mut parse_reply: Option<std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>> =
        None;

    let result = async {
    loop {
        let has_parse_reply = parse_reply.is_some();
        let n = tokio::select! {
            biased;
            _ = connection.closed_token.cancelled() => break,
            _ = async { if let Some(reply) = parse_reply.as_mut() { reply.await; } }, if has_parse_reply => { parse_reply = None; continue; }
            result = buffered.read_until(b'\n', &mut line) => result?,
        };
        if n == 0 && line.is_empty() {
            debug!("ACP: stdin EOF, shutting down reader");
            break;
        }
        let text = std::str::from_utf8(&line).map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?.to_owned();
        line.clear();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                // Spec says: if the message can't be parsed, send a parse-error
                // response with id=null. Best-effort here since we may not even
                // know if it was a request or notification.
                error!(error = %e, "ACP: parse error");
                if let Some(previous) = parse_reply.take() { let _ = previous.await; }
                let reply_connection = connection.clone();
                parse_reply = Some(Box::pin(async move {
                    let _ = reply_connection.send_error_response(acp::RequestId::Null, acp::Error::parse_error()).await;
                }));
                continue;
            }
        };

        // Distinguish response (has id + (result|error)) from request (has id + method)
        // from notification (no id, has method).
        let has_id = v.get("id").is_some();
        let has_method = v.get("method").is_some();
        let has_result = v.get("result").is_some();
        let has_error = v.get("error").is_some();

        if has_id && (has_result || has_error) && !has_method {
            // Response — route to pending.
            let id: acp::RequestId = serde_json::from_value(v["id"].clone()).unwrap_or(acp::RequestId::Null);
            if has_result {
                let value = v["result"].clone();
                connection.complete_pending(&id, Ok(value));
            } else {
                let err: acp::Error = match serde_json::from_value(v["error"].clone()) {
                    Ok(e) => e,
                    Err(_) => acp::Error::internal_error().data(Some(v["error"].clone())),
                };
                connection.complete_pending(&id, Err(err));
            }
        } else if has_id && has_method {
            // Request.
            let id: acp::RequestId = serde_json::from_value(v["id"].clone()).unwrap_or(acp::RequestId::Null);
            let method = v
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let params = v.get("params").cloned();
            if tx
                .send(Inbound::Request { id, method, params })
                .is_err()
            {
                break;
            }
        } else if has_method {
            // Notification.
            let method = v
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let params = v.get("params").cloned();
            if tx.send(Inbound::Notification { method, params }).is_err() {
                break;
            }
        } else {
            warn!(line = %trimmed, "ACP: unrecognized JSON-RPC message shape");
        }
    }
    Ok(())
    }.await;
    connection.close();
    if let Some(reply) = parse_reply {
        let _ = reply.await;
    }
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{duplex, AsyncReadExt};

    /// `send_response` must emit a newline-delimited JSON object containing
    /// `jsonrpc: "2.0"`, the request id, and the result payload.
    #[tokio::test]
    async fn send_response_writes_newline_delimited_jsonrpc() {
        let (client_read, server_write) = duplex(8192);
        let connection = Connection::new(server_write);

        connection
            .send_response(acp::RequestId::Number(7), serde_json::json!({"ok": true}))
            .await
            .unwrap();
        drop(connection);

        let mut buf = Vec::new();
        let mut client_read = client_read;
        client_read.read_to_end(&mut buf).await.unwrap();
        let line = std::str::from_utf8(&buf).unwrap();
        assert!(line.ends_with('\n'), "response missing newline: {line:?}");
        let parsed: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], 7);
        assert_eq!(parsed["result"]["ok"], true);
    }

    /// `send_notification` must omit `id` so the receiver knows not to reply.
    #[tokio::test]
    async fn notifications_have_no_id() {
        let (client_read, server_write) = duplex(8192);
        let connection = Connection::new(server_write);

        connection
            .send_notification("session/update", serde_json::json!({"sessionId": "s1"}))
            .await
            .unwrap();
        drop(connection);

        let mut buf = Vec::new();
        let mut client_read = client_read;
        client_read.read_to_end(&mut buf).await.unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(std::str::from_utf8(&buf).unwrap().trim()).unwrap();
        assert!(parsed.get("id").is_none(), "notification must not carry id");
        assert_eq!(parsed["method"], "session/update");
    }

    /// The reader must distinguish requests, notifications, and responses by
    /// shape and route each to the right callback path.
    #[tokio::test]
    async fn reader_routes_inbound_messages() {
        // Drive the reader from an in-memory pipe.
        let (writer_handle, server_reader) = duplex(8192);
        let (_to_client, client_writer) = duplex(8192);
        let connection = Connection::new(client_writer);
        let (tx, mut rx) = mpsc::unbounded_channel();

        let reader_handle = tokio::spawn(run_reader(connection.clone(), server_reader, tx));

        // Inject a request, a notification, and an unsolicited response.
        let mut writer_handle = writer_handle;
        writer_handle
            .write_all(
                b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":1}}\n",
            )
            .await
            .unwrap();
        writer_handle
            .write_all(
                b"{\"jsonrpc\":\"2.0\",\"method\":\"session/cancel\",\"params\":{\"sessionId\":\"s1\"}}\n",
            )
            .await
            .unwrap();
        writer_handle
            .write_all(
                b"{\"jsonrpc\":\"2.0\",\"id\":99,\"result\":{\"orphan\":true}}\n",
            )
            .await
            .unwrap();
        drop(writer_handle); // EOF the reader

        let first = rx.recv().await.expect("expected request");
        match first {
            Inbound::Request { id, method, .. } => {
                assert_eq!(method, "initialize");
                assert!(matches!(id, acp::RequestId::Number(1)));
            }
            other => panic!("expected Request, got {other:?}"),
        }
        let second = rx.recv().await.expect("expected notification");
        match second {
            Inbound::Notification { method, .. } => {
                assert_eq!(method, "session/cancel");
            }
            other => panic!("expected Notification, got {other:?}"),
        }
        // Orphan response is routed internally; nothing surfaces here.
        assert!(rx.recv().await.is_none(), "no further messages expected");
        let _ = reader_handle.await;
    }

    /// `send_request` resolves when a response with the matching id arrives.
    #[tokio::test]
    async fn send_request_resolves_on_matching_response() {
        let (client_to_server, server_reader) = duplex(8192);
        let (server_to_client_reader, server_to_client) = duplex(8192);
        let connection = Connection::new(server_to_client);
        let (tx, _rx) = mpsc::unbounded_channel();
        let reader_handle =
            tokio::spawn(run_reader(connection.clone(), server_reader, tx));

        // Background: as a fake client, read the outbound request and write a
        // matching response.
        let client_loop = tokio::spawn(async move {
            let mut buf = Vec::new();
            let mut server_to_client_reader = server_to_client_reader;
            let mut byte = [0u8; 1];
            // Read up to one full line.
            loop {
                let n = server_to_client_reader.read(&mut byte).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.push(byte[0]);
                if byte[0] == b'\n' {
                    break;
                }
            }
            let outbound: serde_json::Value =
                serde_json::from_slice(buf.trim_ascii_end()).unwrap();
            let id = outbound["id"].clone();
            // Send the response back through the client_to_server pipe.
            let response = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {"echo": outbound["method"].as_str().unwrap()}
            });
            let mut bytes = serde_json::to_vec(&response).unwrap();
            bytes.push(b'\n');
            let mut client_to_server = client_to_server;
            client_to_server.write_all(&bytes).await.unwrap();
            drop(client_to_server);
        });

        let response: Result<serde_json::Value, acp::Error> = connection
            .send_request("fs/read_text_file", serde_json::json!({"path": "/x"}))
            .await
            .unwrap();
        let response = response.unwrap();
        assert_eq!(response["echo"], "fs/read_text_file");
        let _ = client_loop.await;
        let _ = reader_handle.await;
    }
}

#[cfg(test)]
mod stdio_slice_tests {
    use super::*;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::io::{duplex, AsyncReadExt, AsyncWrite};

    struct PartialErrorWriter {
        call: usize,
    }
    impl AsyncWrite for PartialErrorWriter {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            self.call += 1;
            match self.call {
                1 => Poll::Ready(Ok(bytes.len().min(3))),
                2 => Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "scripted error",
                ))),
                _ => Poll::Ready(Ok(bytes.len())),
            }
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    struct ChunkedInput(std::io::Cursor<Vec<u8>>);
    impl std::io::Read for ChunkedInput {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let limit = buf.len().min(3);
            std::io::Read::read(&mut self.0, &mut buf[..limit])
        }
    }

    #[tokio::test]
    async fn threaded_input_preserves_partial_frames_through_eof() {
        let bytes = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"first\"}\n{\"jsonrpc\":\"2.0\",\"method\":\"second\"}\n".to_vec();
        let (input, thread) = threaded_input(ChunkedInput(std::io::Cursor::new(bytes)));
        let (_client, server) = duplex(128);
        let connection = Connection::new(server);
        let (tx, mut rx) = mpsc::unbounded_channel();
        run_reader(connection.clone(), input, tx).await.unwrap();
        thread.join().unwrap();
        assert!(
            matches!(rx.recv().await, Some(Inbound::Request { method, .. }) if method == "first")
        );
        assert!(
            matches!(rx.recv().await, Some(Inbound::Notification { method, .. }) if method == "second")
        );
        assert!(rx.recv().await.is_none());
        assert!(connection.closed_token.is_cancelled());
    }

    struct SignalledInput {
        reads: usize,
        second_read: Option<std::sync::mpsc::Sender<()>>,
    }
    impl std::io::Read for SignalledInput {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.reads += 1;
            if self.reads == 2 {
                if let Some(sender) = self.second_read.take() {
                    let _ = sender.send(());
                }
            }
            buf[0] = b'x';
            Ok(1)
        }
    }

    #[test]
    fn threaded_input_receiver_closure_releases_blocked_sender() {
        let (second_tx, second_rx) = std::sync::mpsc::channel();
        let (input, thread) = threaded_input(SignalledInput {
            reads: 0,
            second_read: Some(second_tx),
        });
        second_rx.recv().unwrap();
        drop(input);
        thread.join().unwrap();
    }

    struct FragmentedOutput {
        bytes: Arc<parking_lot::Mutex<Vec<u8>>>,
        fail_after_first: bool,
        wrote: bool,
    }
    impl std::io::Write for FragmentedOutput {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.fail_after_first && self.wrote {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "scripted failure",
                ));
            }
            let count = bytes.len().min(3);
            self.bytes.lock().extend_from_slice(&bytes[..count]);
            self.wrote = true;
            Ok(count)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn threaded_output_flush_waits_for_all_fragmented_bytes() {
        let bytes = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let (mut writer, thread) = threaded_output(FragmentedOutput {
            bytes: bytes.clone(),
            fail_after_first: false,
            wrote: false,
        });
        let data = vec![b'x'; 20001];
        writer.write_all(&data).await.unwrap();
        writer.flush().await.unwrap();
        assert_eq!(*bytes.lock(), data);
        drop(writer);
        thread.join().unwrap();
    }

    #[tokio::test]
    async fn threaded_output_reports_error_after_partial_write() {
        let bytes = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let (mut writer, thread) = threaded_output(FragmentedOutput {
            bytes: bytes.clone(),
            fail_after_first: true,
            wrote: false,
        });
        writer.write_all(b"abcdef").await.unwrap();
        assert!(writer.flush().await.is_err());
        assert_eq!(&*bytes.lock(), b"abc");
        drop(writer);
        thread.join().unwrap();
    }

    struct ObservedReader<R> {
        inner: R,
        seen: Option<tokio::sync::oneshot::Sender<()>>,
    }
    impl<R: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for ObservedReader<R> {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let start = buf.filled().len();
            let result = Pin::new(&mut self.inner).poll_read(cx, buf);
            if buf.filled()[start..]
                .windows(4)
                .any(|bytes| bytes == b"abc-")
            {
                if let Some(seen) = self.seen.take() {
                    let _ = seen.send(());
                }
            }
            result
        }
    }

    #[tokio::test]
    async fn partial_input_survives_parse_reply_wakeup() {
        use tokio::io::AsyncWriteExt;
        let (mut client_input, server_input) = duplex(8192);
        let (mut client_output, server_output) = duplex(1);
        let connection = Connection::new(server_output);
        let (seen_tx, seen_rx) = oneshot::channel();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let reader = tokio::spawn(run_reader(
            connection,
            ObservedReader {
                inner: server_input,
                seen: Some(seen_tx),
            },
            tx,
        ));
        client_input.write_all(b"{bad}\n").await.unwrap();
        let mut first = [0u8; 1];
        client_output.read_exact(&mut first).await.unwrap();
        client_input
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"abc-")
            .await
            .unwrap();
        seen_rx.await.unwrap();
        let mut byte = [0u8; 1];
        loop {
            client_output.read_exact(&mut byte).await.unwrap();
            if byte[0] == b'\n' {
                break;
            }
        }
        client_input.write_all(b"done\"}\n").await.unwrap();
        assert!(
            matches!(rx.recv().await, Some(Inbound::Request { method, .. }) if method == "abc-done")
        );
        drop(client_input);
        reader.await.unwrap().unwrap();
    }

    struct PendingWriter;
    impl AsyncWrite for PendingWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Poll::Pending
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Pending
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Pending
        }
    }

    #[tokio::test]
    async fn writable_parse_error_reply_is_emitted_before_eof() {
        use tokio::io::AsyncWriteExt;
        let (mut client_input, server_input) = duplex(8192);
        let (mut client_output, server_output) = duplex(8192);
        let connection = Connection::new(server_output);
        let (tx, _rx) = mpsc::unbounded_channel();
        client_input.write_all(b"{bad json}\n").await.unwrap();
        drop(client_input);
        run_reader(connection, server_input, tx).await.unwrap();
        let mut bytes = [0u8; 8192];
        let length = client_output.read(&mut bytes).await.unwrap();
        let reply: Value = serde_json::from_slice(&bytes[..length]).unwrap();
        assert_eq!(reply["error"]["code"], -32700);
    }

    #[tokio::test]
    async fn multiple_malformed_lines_each_get_a_reply_while_writable() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let (mut client_input, server_input) = duplex(8192);
        let (client_output, server_output) = duplex(8192);
        let connection = Connection::new(server_output);
        let (tx, _rx) = mpsc::unbounded_channel();
        let reader = tokio::spawn(run_reader(connection, server_input, tx));
        client_input
            .write_all(b"{bad}\n{bad again}\n")
            .await
            .unwrap();
        let mut output = BufReader::new(client_output);
        for _ in 0..2 {
            let mut line = String::new();
            tokio::time::timeout(
                std::time::Duration::from_secs(1),
                output.read_line(&mut line),
            )
            .await
            .unwrap()
            .unwrap();
            let reply: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(reply["error"]["code"], -32700);
        }
        drop(client_input);
        reader.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn eof_is_observed_while_parse_error_reply_is_backpressured() {
        use tokio::io::AsyncWriteExt;
        let (mut client, server) = duplex(8192);
        let connection = Connection::new(PendingWriter);
        let (tx, _rx) = mpsc::unbounded_channel();
        let reader = tokio::spawn(run_reader(connection, server, tx));
        client.write_all(b"{bad json}\n").await.unwrap();
        drop(client);
        tokio::time::timeout(std::time::Duration::from_secs(1), reader)
            .await
            .expect("EOF was blocked by parse reply")
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn partial_frame_failure_poison_connection_for_later_writes() {
        let connection = Connection::new(PartialErrorWriter { call: 0 });
        assert!(connection.send_notification("first", ()).await.is_err());
        assert!(connection.send_notification("second", ()).await.is_err());
    }

    #[tokio::test]
    async fn input_eof_resolves_pending_permission_request() {
        let (mut client_output, server_output) = duplex(8192);
        let (client_input, server_input) = duplex(8192);
        let connection = Connection::new(server_output);
        let (tx, _rx) = mpsc::unbounded_channel();
        let read = tokio::spawn(run_reader(connection.clone(), server_input, tx));
        let request_connection = connection.clone();
        let request = tokio::spawn(async move {
            request_connection
                .send_request::<_, Value>("session/request_permission", ())
                .await
        });
        let mut buffer = [0u8; 8192];
        assert!(client_output.read(&mut buffer).await.unwrap() > 0);
        drop(client_input);
        read.await.unwrap().unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), request).await;
        assert!(result
            .expect("pending request survived EOF")
            .unwrap()
            .is_err());
    }
}
