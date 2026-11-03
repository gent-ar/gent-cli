use std::collections::BTreeMap;
use std::io::{BufRead, Read, Write};

use super::EgressError;

const MAX_HEADERS: usize = 16 * 1024;
const MAX_BODY: usize = 16 * 1024 * 1024;

pub(super) struct Request {
    pub method: String,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

fn line(reader: &mut impl BufRead, total: &mut usize) -> Result<String, EgressError> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_HEADERS + 1 - *total) as u64)
        .read_until(b'\n', &mut bytes)
        .map_err(|_| EgressError::Request)?;
    *total += bytes.len();
    if *total > MAX_HEADERS || !bytes.ends_with(b"\r\n") {
        return Err(EgressError::Request);
    }
    bytes.truncate(bytes.len() - 2);
    String::from_utf8(bytes).map_err(|_| EgressError::Request)
}

pub(super) fn read(
    reader: &mut impl BufRead,
    authority: &str,
    token: &str,
) -> Result<Request, EgressError> {
    let mut total = 0;
    let start = line(reader, &mut total)?;
    let parts: Vec<_> = start.split(' ').collect();
    if parts.len() != 3 || parts[2] != "HTTP/1.1" || !matches!(parts[0], "GET" | "POST") {
        return Err(EgressError::Request);
    }
    if !parts[1].starts_with('/')
        || parts[1].starts_with("//")
        || parts[1].contains(['#', '\\', '%'])
    {
        return Err(EgressError::Request);
    }
    let mut headers = BTreeMap::new();
    loop {
        let field = line(reader, &mut total)?;
        if field.is_empty() {
            break;
        }
        let (name, value) = field.split_once(':').ok_or(EgressError::Request)?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || value.bytes().any(|byte| byte < 32 || byte == 127)
        {
            return Err(EgressError::Request);
        }
        if headers
            .insert(name.to_ascii_lowercase(), value.trim().to_string())
            .is_some()
        {
            return Err(EgressError::Request);
        }
    }
    if headers.get("host").map(String::as_str) != Some(authority)
        || headers.contains_key("transfer-encoding")
        || headers.contains_key("upgrade")
        || headers.contains_key("expect")
        || headers.contains_key("proxy-authorization")
    {
        return Err(EgressError::Request);
    }
    let authorized = match (headers.get("authorization"), headers.get("x-api-key")) {
        (Some(bearer), None) => equal(bearer.as_bytes(), format!("Bearer {token}").as_bytes()),
        (None, Some(key)) => equal(key.as_bytes(), token.as_bytes()),
        _ => false,
    };
    if !authorized {
        return Err(EgressError::Authentication);
    }
    let length = match headers.get("content-length") {
        Some(value) if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) => {
            value.parse::<usize>().map_err(|_| EgressError::Request)?
        }
        None if parts[0] == "GET" => 0,
        _ => return Err(EgressError::Request),
    };
    if length > MAX_BODY || (parts[0] == "GET" && length != 0) {
        return Err(EgressError::Request);
    }
    if parts[0] == "POST"
        && headers
            .get("content-type")
            .is_none_or(|value| value.split(';').next() != Some("application/json"))
    {
        return Err(EgressError::Request);
    }
    let mut body = vec![0; length];
    reader
        .read_exact(&mut body)
        .map_err(|_| EgressError::Request)?;
    Ok(Request {
        method: parts[0].into(),
        path: parts[1].into(),
        headers,
        body,
    })
}

fn equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0, |difference, (a, b)| difference | (a ^ b))
        == 0
}

pub(super) fn response(
    mut upstream: reqwest::blocking::Response,
    output: &mut impl Write,
) -> Result<(), EgressError> {
    write!(
        output,
        "HTTP/1.1 {} Upstream\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n",
        upstream.status().as_u16()
    )
    .map_err(|_| EgressError::Transport)?;
    for name in [
        "content-type",
        "content-encoding",
        "request-id",
        "x-request-id",
        "retry-after",
    ] {
        if let Some(value) = upstream.headers().get(name) {
            let value = value.to_str().map_err(|_| EgressError::Transport)?;
            write!(output, "{name}: {value}\r\n").map_err(|_| EgressError::Transport)?;
        }
    }
    output
        .write_all(b"\r\n")
        .map_err(|_| EgressError::Transport)?;
    let mut chunk = [0; 16 * 1024];
    loop {
        let count = upstream
            .read(&mut chunk)
            .map_err(|_| EgressError::Transport)?;
        if count == 0 {
            output
                .write_all(b"0\r\n\r\n")
                .map_err(|_| EgressError::Transport)?;
            return Ok(());
        }
        write!(output, "{count:x}\r\n").map_err(|_| EgressError::Transport)?;
        output
            .write_all(&chunk[..count])
            .map_err(|_| EgressError::Transport)?;
        output
            .write_all(b"\r\n")
            .map_err(|_| EgressError::Transport)?;
        output.flush().map_err(|_| EgressError::Transport)?;
    }
}

pub(super) fn reject(output: &mut impl Write, error: &EgressError) {
    let status = match error {
        EgressError::Authentication => "401 Unauthorized",
        EgressError::CredentialProbe {
            phase: super::CredentialProbePhase::LoginUnavailable,
            ..
        } => "401 Unauthorized",
        EgressError::CredentialProbe { .. } => "503 Service Unavailable",
        EgressError::Transport => "502 Bad Gateway",
        _ => "400 Bad Request",
    };
    let _ = write!(
        output,
        "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    );
}
