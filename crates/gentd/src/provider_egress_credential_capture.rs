use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Cursor, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{EgressError, wire};

pub(super) struct CapturedProviderAuth {
    pub headers: Vec<(String, String)>,
}

impl std::fmt::Debug for CapturedProviderAuth {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CapturedProviderAuth")
            .finish_non_exhaustive()
    }
}

pub(super) struct CredentialCapture {
    pub base_url: String,
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<Result<CapturedProviderAuth, EgressError>>>,
}

impl CredentialCapture {
    pub fn start() -> Result<Self, EgressError> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| EgressError::Authentication)?;
        listener
            .set_nonblocking(true)
            .map_err(|_| EgressError::Authentication)?;
        let authority = listener
            .local_addr()
            .map_err(|_| EgressError::Authentication)?
            .to_string();
        let nonce = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let base_url = format!("http://{authority}/{nonce}");
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(45);
            let mut hello_seen = false;
            while Instant::now() < deadline && !stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, peer)) if peer.ip().is_loopback() => {
                        stream
                            .set_nonblocking(false)
                            .map_err(|_| EgressError::Authentication)?;
                        stream
                            .set_read_timeout(Some(Duration::from_secs(10)))
                            .map_err(|_| EgressError::Authentication)?;
                        stream
                            .set_write_timeout(Some(Duration::from_secs(10)))
                            .map_err(|_| EgressError::Authentication)?;
                        match read(&mut stream, &authority, &nonce, !hello_seen)? {
                            Some((auth, model)) => {
                                respond(&mut stream, &model)?;
                                return Ok(auth);
                            }
                            None => hello_seen = true,
                        }
                    }
                    Ok(_) => return Err(EgressError::Authentication),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => return Err(EgressError::Authentication),
                }
            }
            Err(EgressError::Authentication)
        });
        Ok(Self {
            base_url,
            stopped,
            worker: Some(worker),
        })
    }

    pub fn finish(mut self) -> Result<CapturedProviderAuth, EgressError> {
        self.stopped.store(true, Ordering::Release);
        self.worker
            .take()
            .ok_or(EgressError::Authentication)?
            .join()
            .map_err(|_| EgressError::Authentication)?
    }
}

impl Drop for CredentialCapture {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn read(
    stream: &mut TcpStream,
    authority: &str,
    nonce: &str,
    allow_hello: bool,
) -> Result<Option<(CapturedProviderAuth, String)>, EgressError> {
    let mut reader = BufReader::new(&mut *stream);
    let mut lines = Vec::new();
    let mut total = 0;
    loop {
        let mut line = Vec::new();
        reader
            .by_ref()
            .take((16385 - total) as u64)
            .read_until(b'\n', &mut line)
            .map_err(|_| EgressError::Authentication)?;
        total += line.len();
        if total > 16384 || !line.ends_with(b"\r\n") {
            return Err(EgressError::Authentication);
        }
        if line == b"\r\n" {
            break;
        }
        lines.push(String::from_utf8(line).map_err(|_| EgressError::Authentication)?);
    }
    let first = lines.first().ok_or(EgressError::Authentication)?;
    let start = format!("POST /{nonce}/v1/messages HTTP/1.1\r\n");
    let beta = format!("POST /{nonce}/v1/messages?beta=true HTTP/1.1\r\n");
    let hello = first == &format!("HEAD /{nonce}/api/hello HTTP/1.1\r\n");
    if first != &start && first != &beta && !(hello && allow_hello) {
        return Err(EgressError::Authentication);
    }
    let mut names = BTreeSet::new();
    let mut headers = BTreeMap::new();
    let mut rebuilt = first.as_bytes().to_vec();
    for line in &lines[1..] {
        let (name, value) = line
            .trim_end_matches("\r\n")
            .split_once(':')
            .ok_or(EgressError::Authentication)?;
        let name = name.to_ascii_lowercase();
        if !names.insert(name.clone()) {
            return Err(EgressError::Authentication);
        }
        let value = value.trim();
        headers.insert(name.clone(), value.to_string());
        if name != "authorization" {
            rebuilt.extend_from_slice(line.as_bytes());
        }
    }
    if hello {
        // Exact nonce-bound local preflight, never credential/policy authority.
        if headers.get("host").map(String::as_str) != Some(authority)
            || headers.keys().any(|name| {
                !matches!(
                    name.as_str(),
                    "host"
                        | "accept"
                        | "accept-encoding"
                        | "connection"
                        | "user-agent"
                        | "content-length"
                )
            })
            || headers
                .get("content-length")
                .is_some_and(|length| length != "0")
        {
            return Err(EgressError::Authentication);
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .map_err(|_| EgressError::Authentication)?;
        return Ok(None);
    }
    let authorization = headers
        .get("authorization")
        .filter(|value| value.starts_with("Bearer ") && value.len() > 8)
        .ok_or(EgressError::Authentication)?;
    if authorization == "Bearer gent-dummy-gateway-token" {
        return Err(EgressError::Authentication);
    }
    rebuilt.extend_from_slice(b"Authorization: Bearer probe\r\n\r\n");
    let length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|length| *length <= 2 * 1024 * 1024)
        .ok_or(EgressError::Authentication)?;
    let mut body = vec![0; length];
    reader
        .read_exact(&mut body)
        .map_err(|_| EgressError::Authentication)?;
    rebuilt.extend_from_slice(&body);
    let request = wire::read(&mut Cursor::new(rebuilt), authority, "probe")?;
    let body: Value =
        serde_json::from_slice(&request.body).map_err(|_| EgressError::Authentication)?;
    if body
        .get("tools")
        .is_some_and(|tools| tools.as_array().is_none_or(|tools| !tools.is_empty()))
        || body.get("stream") != Some(&Value::Bool(true))
    {
        return Err(EgressError::Authentication);
    }
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .ok_or(EgressError::Authentication)?
        .to_string();
    let headers = headers
        .into_iter()
        .filter(|(name, _)| {
            matches!(
                name.as_str(),
                "authorization"
                    | "anthropic-beta"
                    | "anthropic-version"
                    | "chatgpt-account-id"
                    | "openai-beta"
                    | "originator"
                    | "user-agent"
            )
        })
        .collect();
    Ok(Some((CapturedProviderAuth { headers }, model)))
}

fn respond(stream: &mut TcpStream, model: &str) -> Result<(), EgressError> {
    let message = json!({"id":"msg_gent_credential_probe","type":"message","role":"assistant","model":model,
        "content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":0}});
    let events = [
        json!({"type":"message_start","message":message}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"OK"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":1}}),
        json!({"type":"message_stop"}),
    ];
    let body = events.iter().fold(String::new(), |mut body, event| {
        use std::fmt::Write as _;
        let _ = write!(
            body,
            "event: {}\ndata: {event}\n\n",
            event["type"].as_str().unwrap_or("error")
        );
        body
    });
    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).map_err(|_| EgressError::Authentication)
}

#[cfg(test)]
#[path = "provider_egress_credential_capture_tests.rs"]
mod tests;
