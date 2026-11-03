use super::{ModelError, Scope};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    io::Write,
    sync::Arc,
    time::{Duration, Instant},
};
const MAX_FRAME: usize = 32768;
#[derive(Clone, Debug)]
pub(crate) struct ModelRequest {
    pub method: String,
    pub path: String,
    pub body: Vec<u8>,
}
impl ModelRequest {
    pub(super) fn encode(&self) -> Result<Vec<u8>, ModelError> {
        if !matches!(
            (self.method.as_str(), self.path.as_str()),
            ("POST", "/v1/chat/completions") | ("GET", "/v1/models") | ("GET", "/health")
        ) || self.body.len() > 131_072
        {
            return Err(ModelError::Request);
        }
        let body = std::str::from_utf8(&self.body).map_err(|_| ModelError::Request)?;
        let json = serde_json::to_vec(
            &serde_json::json!({"method":self.method,"path":self.path,"body":body}),
        )
        .map_err(|_| ModelError::Request)?;
        if json.len() > 262_144 {
            return Err(ModelError::Request);
        }
        let mut out = u32::try_from(json.len())
            .map_err(|_| ModelError::Request)?
            .to_le_bytes()
            .to_vec();
        out.extend(json);
        Ok(out)
    }
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Metadata {
    status: u16,
    content_type: String,
    headers: BTreeMap<String, String>,
    stream: bool,
}
impl Metadata {
    pub fn parse(bytes: &[u8]) -> Result<Self, ModelError> {
        let value: Self = serde_json::from_slice(bytes).map_err(|_| ModelError::Protocol)?;
        if !(200..600).contains(&value.status)
            || (300..400).contains(&value.status)
            || value.content_type.is_empty()
            || value.content_type.len() > 128
            || !visible(&value.content_type)
            || value.headers.len() > 32
        {
            return Err(ModelError::Protocol);
        }
        for (name, value) in &value.headers {
            if name.is_empty()
                || name.len() > 128
                || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                || value.len() > 4096
                || !visible(value)
            {
                return Err(ModelError::Protocol);
            }
        }
        if value.stream && !value.content_type.starts_with("text/event-stream") {
            return Err(ModelError::Protocol);
        }
        Ok(value)
    }
    pub fn write(&self, out: &mut impl Write) -> Result<(), ModelError> {
        write!(out,"HTTP/1.1 {} Model\r\nConnection: close\r\nTransfer-Encoding: chunked\r\nContent-Type: {}\r\n",self.status,self.content_type).map_err(|_|ModelError::Transport)?;
        for (name, value) in &self.headers {
            if matches!(
                name.to_ascii_lowercase().as_str(),
                "request-id" | "x-request-id" | "retry-after"
            ) {
                write!(out, "{name}: {value}\r\n").map_err(|_| ModelError::Transport)?;
            }
        }
        out.write_all(b"\r\n").map_err(|_| ModelError::Transport)
    }
}
fn visible(s: &str) -> bool {
    s.bytes().all(|b| (32..127).contains(&b))
}
pub(super) struct Reader {
    scope: Arc<Scope>,
    buffer: Vec<u8>,
    at: usize,
}
impl Reader {
    pub fn new(scope: Arc<Scope>) -> Self {
        Self {
            scope,
            buffer: Vec::new(),
            at: 0,
        }
    }
    pub fn exact(&mut self, length: usize, deadline: Instant) -> Result<Vec<u8>, ModelError> {
        if length > MAX_FRAME {
            return Err(ModelError::Protocol);
        }
        while self.buffer.len() - self.at < length {
            if !self.scope.current() {
                return Err(ModelError::Revoked);
            }
            if Instant::now() >= deadline {
                return Err(ModelError::Timeout);
            }
            match self
                .scope
                .process
                .next_stdout_chunk()
                .map_err(|_| ModelError::Transport)?
            {
                Some(chunk) => {
                    if chunk.is_empty() || self.buffer.len() - self.at + chunk.len() > 1024 * 1024 {
                        return Err(ModelError::Protocol);
                    }
                    self.buffer.drain(..self.at);
                    self.at = 0;
                    self.buffer.extend(chunk);
                }
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        let value = self.buffer[self.at..self.at + length].to_vec();
        self.at += length;
        Ok(value)
    }
    pub fn frame(&mut self, deadline: Instant) -> Result<Vec<u8>, ModelError> {
        let raw = self.exact(4, deadline)?;
        let count = u32::from_le_bytes(raw.try_into().map_err(|_| ModelError::Protocol)?) as usize;
        self.exact(count, deadline)
    }
}
pub(super) fn chunk(out: &mut impl Write, body: &[u8]) -> Result<(), ModelError> {
    write!(out, "{:x}\r\n", body.len())
        .and_then(|()| out.write_all(body))
        .and_then(|()| out.write_all(b"\r\n"))
        .and_then(|()| out.flush())
        .map_err(|_| ModelError::Transport)
}

// The actor itself authored this HTTP framing; still require exact bounds and
// completion before allowing a daemon-owned context summary to consume JSON.
pub(super) fn json_response(response: &[u8]) -> Result<serde_json::Value, ModelError> {
    let end = response
        .windows(4)
        .position(|b| b == b"\r\n\r\n")
        .ok_or(ModelError::Protocol)?;
    if !response.starts_with(b"HTTP/1.1 200 ") {
        return Err(ModelError::Protocol);
    }
    let mut rest = &response[end + 4..];
    let mut body = Vec::new();
    loop {
        let line = rest
            .windows(2)
            .position(|b| b == b"\r\n")
            .ok_or(ModelError::Protocol)?;
        if line == 0 || line > 8 {
            return Err(ModelError::Protocol);
        }
        let count = usize::from_str_radix(
            std::str::from_utf8(&rest[..line]).map_err(|_| ModelError::Protocol)?,
            16,
        )
        .map_err(|_| ModelError::Protocol)?;
        rest = &rest[line + 2..];
        if count == 0 {
            if rest != b"\r\n" {
                return Err(ModelError::Protocol);
            }
            break;
        }
        if count > 32768
            || body.len() + count > 4 * 1024 * 1024
            || rest.len() < count + 2
            || &rest[count..count + 2] != b"\r\n"
        {
            return Err(ModelError::Protocol);
        }
        body.extend_from_slice(&rest[..count]);
        rest = &rest[count + 2..];
    }
    serde_json::from_slice(&body).map_err(|_| ModelError::Protocol)
}
