//! Fixed-destination HTTP proxy. The caller retains the real daemon lease and
//! supplies its live RunFence check; an endpoint/token alone is never authority.
use std::{
    io::{Read, Write},
    net::{SocketAddrV4, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
pub trait LiveGatewayLease: Send + Sync + 'static {
    fn current(&self) -> bool;
}
/// A daemon lease AND the exact live helper/cgroup owner are both mandatory.
/// Weak ownership makes gateway activity unable to prolong a terminated run.
pub struct OwnerBoundLease {
    pub owner: std::sync::Weak<std::sync::Mutex<crate::linux_cgroup_contract::ScopedHelperOwner>>,
    pub daemon: Arc<dyn LiveGatewayLease>,
}
impl LiveGatewayLease for OwnerBoundLease {
    fn current(&self) -> bool {
        if !self.daemon.current() {
            return false;
        }
        let Some(owner) = self.owner.upgrade() else {
            return false;
        };
        owner
            .lock()
            .map(|mut owner| owner.gateway_owner_current())
            .unwrap_or(false)
    }
}
pub struct GatewayBridge {
    _listener: TcpListener,
    lease: Arc<dyn LiveGatewayLease>,
    alive: Arc<AtomicBool>,
    diagnostic: Arc<std::sync::Mutex<std::collections::BTreeMap<String, u64>>>,
    endpoint: SocketAddrV4,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl GatewayBridge {
    pub fn start(
        target: SocketAddrV4,
        lease: Arc<dyn LiveGatewayLease>,
    ) -> Result<Self, &'static str> {
        if !target.ip().is_loopback() || target.port() == 0 || !lease.current() {
            return Err("invalid gateway lease");
        }
        let socket = TcpListener::bind("127.0.0.1:0").map_err(|_| "bridge bind")?;
        socket.set_nonblocking(true).map_err(|_| "bridge flags")?;
        let std::net::SocketAddr::V4(endpoint) =
            socket.local_addr().map_err(|_| "bridge address")?
        else {
            return Err("bridge address");
        };
        let diagnostic = Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::new()));
        let observed = diagnostic.clone();
        let retained = socket
            .try_clone()
            .map_err(|_| "bridge listener ownership")?;
        let alive = Arc::new(AtomicBool::new(true));
        let life = alive.clone();
        let retained_lease = lease.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let cancelled = stop.clone();
        // A finite pool supports simultaneous provider streams/MCP HTTP channels.
        // The owner joins every accepted connection; no detached worker survives Drop.
        let owner = thread::spawn(move || {
            let owned_life = life.clone();
            let _life = DispatcherLife {
                alive: life,
                cancelled: cancelled.clone(),
            };
            let mut workers = OwnedWorkers {
                handles: Vec::new(),
                alive: owned_life,
                cancelled: cancelled.clone(),
            };
            while !cancelled.load(Ordering::Acquire) && lease.current() {
                let mut at = 0;
                while at < workers.len() {
                    if workers[at].is_finished() {
                        let worker = workers.swap_remove(at);
                        let _ = worker.join();
                    } else {
                        at += 1;
                    }
                }
                match socket.accept() {
                    Ok((mut stream, _)) => {
                        if workers.len() >= 16 {
                            continue;
                        }
                        let lease = lease.clone();
                        let cancelled = cancelled.clone();
                        let observed = observed.clone();
                        workers.push(thread::spawn(move || {
                            let _ = exchange(&mut stream, target, &*lease, &cancelled, &observed);
                        }));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(_) => break,
                }
            }
            cancelled.store(true, Ordering::Release);
            for worker in workers.drain(..) {
                let _ = worker.join();
            }
        });
        Ok(Self {
            _listener: retained,
            lease: retained_lease,
            alive,
            diagnostic,
            endpoint,
            stop,
            thread: Some(owner),
        })
    }
    pub fn endpoint(&self) -> SocketAddrV4 {
        self.endpoint
    }
    pub fn current(&self) -> bool {
        self.alive.load(Ordering::Acquire)
            && !self.stop.load(Ordering::Acquire)
            && self.lease.current()
    }
    pub(crate) fn cancellation(&self) -> Arc<AtomicBool> {
        self.stop.clone()
    }
    pub(crate) fn liveness(&self) -> Arc<AtomicBool> {
        self.alive.clone()
    }
    pub fn diagnostic(&self) -> std::collections::BTreeMap<String, u64> {
        self.diagnostic.lock().unwrap().clone()
    }
}
// The dispatcher also joins on unwinding; dropping JoinHandles alone would detach.
struct OwnedWorkers {
    handles: Vec<thread::JoinHandle<()>>,
    alive: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
}
impl std::ops::Deref for OwnedWorkers {
    type Target = Vec<thread::JoinHandle<()>>;
    fn deref(&self) -> &Self::Target {
        &self.handles
    }
}
impl std::ops::DerefMut for OwnedWorkers {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.handles
    }
}
impl Drop for OwnedWorkers {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
        self.cancelled.store(true, Ordering::Release);
        for worker in self.handles.drain(..) {
            let _ = worker.join();
        }
    }
}
struct DispatcherLife {
    alive: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
}
impl Drop for DispatcherLife {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
        self.cancelled.store(true, Ordering::Release);
    }
}
impl Drop for GatewayBridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(owner) = self.thread.take() {
            let _ = owner.join();
        }
    }
}
fn request_head(
    head: &[u8],
    target: SocketAddrV4,
) -> Result<(Vec<u8>, Option<usize>), &'static str> {
    let text = std::str::from_utf8(head).map_err(|_| "header encoding")?;
    if !text.ends_with("\r\n\r\n") {
        return Err("header framing");
    }
    let mut lines = text[..text.len() - 4].split("\r\n");
    let first = lines.next().ok_or("request line")?;
    let fields = first.split(' ').collect::<Vec<_>>();
    if fields.len() != 3 || !matches!(fields[0], "GET" | "POST" | "HEAD") || fields[2] != "HTTP/1.1"
    {
        return Err("request method");
    }
    let prefix = format!("http://{target}");
    let path = fields[1].strip_prefix(&prefix).ok_or("foreign origin")?;
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.bytes().any(|b| b <= 32 || b == 127 || b == b'#')
    {
        return Err("request path");
    }
    let mut content_length = None;
    let mut chunked = false;
    let mut host = false;
    let mut proxy_connection = false;
    let mut result = format!("{} {} HTTP/1.1\r\n", fields[0], path);
    for line in lines {
        if line.starts_with(' ') || line.starts_with('\t') {
            return Err("folded header");
        }
        let (name, value) = line.split_once(':').ok_or("header syntax")?;
        if name.is_empty()
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || value.bytes().any(|b| b < 32 && b != b'\t' || b == 127)
        {
            return Err("header syntax");
        }
        let lower = name.to_ascii_lowercase();
        let value = value.trim();
        match lower.as_str() {
            "host" => {
                if host || value != target.to_string() {
                    return Err("host mismatch");
                }
                host = true;
            }
            "content-length" => {
                if content_length.is_some()
                    || value.is_empty()
                    || !value.bytes().all(|b| b.is_ascii_digit())
                {
                    return Err("body framing");
                }
                let len = value.parse::<usize>().map_err(|_| "body framing")?;
                if len > 8 * 1024 * 1024 {
                    return Err("body bound");
                }
                content_length = Some(len);
            }
            "transfer-encoding" => {
                if chunked || !value.eq_ignore_ascii_case("chunked") {
                    return Err("unsupported transfer encoding");
                }
                chunked = true;
            }
            "upgrade" => return Err("upgrade header"),
            "trailer" => return Err("trailer header"),
            "proxy-authorization" => return Err("proxy authorization header"),
            "proxy-connection" => {
                if proxy_connection
                    || (!value.eq_ignore_ascii_case("keep-alive")
                        && !value.eq_ignore_ascii_case("close"))
                {
                    return Err("invalid proxy connection header");
                }
                proxy_connection = true;
                continue;
            }
            "connection" => {
                if !value.eq_ignore_ascii_case("close") && !value.eq_ignore_ascii_case("keep-alive")
                {
                    return Err("connection options");
                }
                continue;
            }
            _ => {}
        }
        result.push_str(line);
        result.push_str("\r\n");
    }
    if !host {
        return Err("missing host");
    }
    if chunked && content_length.is_some() {
        return Err("ambiguous framing");
    }
    result.push_str("Connection: close\r\n\r\n");
    Ok((
        result.into_bytes(),
        if chunked {
            None
        } else {
            Some(content_length.unwrap_or(0))
        },
    ))
}
fn exchange(
    client: &mut TcpStream,
    target: SocketAddrV4,
    lease: &dyn LiveGatewayLease,
    stop: &AtomicBool,
    diagnostic: &std::sync::Mutex<std::collections::BTreeMap<String, u64>>,
) -> Result<(), &'static str> {
    let timeout = Some(Duration::from_millis(100));
    client.set_read_timeout(timeout).map_err(|_| "socket")?;
    client.set_write_timeout(timeout).map_err(|_| "socket")?;
    let deadline = std::time::Instant::now() + Duration::from_secs(600);
    let live =
        || !stop.load(Ordering::Acquire) && lease.current() && std::time::Instant::now() < deadline;
    let mut head = Vec::new();
    let mut byte = [0; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if !live() || head.len() >= 32768 {
            return Err("header bound or lease");
        }
        match client.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            _ => return Err("header read"),
        }
    }
    let method = head.split(|b| *b == b' ').next().unwrap_or_default();
    let key = match method {
        b"CONNECT" => "CONNECT",
        b"POST" => "POST",
        b"GET" => "GET",
        b"HEAD" => "HEAD",
        _ => "OTHER",
    };
    *diagnostic
        .lock()
        .unwrap()
        .entry(format!("method:{key}"))
        .or_default() += 1;
    let (head, body) = request_head(&head, target).map_err(|reason| {
        *diagnostic
            .lock()
            .unwrap()
            .entry(format!("reject:{reason}"))
            .or_default() += 1;
        reason
    })?;
    if !live() {
        return Err("stale lease");
    }
    let mut upstream = TcpStream::connect_timeout(&target.into(), Duration::from_millis(250))
        .map_err(|_| "upstream connect")?;
    upstream.set_read_timeout(timeout).map_err(|_| "socket")?;
    upstream.set_write_timeout(timeout).map_err(|_| "socket")?;
    upstream.write_all(&head).map_err(|_| "upstream header")?;
    let mut buffer = [0; 16384];
    let mut copy_exact = |mut size: usize| -> Result<(), &'static str> {
        while size > 0 {
            if !live() {
                return Err("stale lease");
            }
            let count = size.min(buffer.len());
            match client.read(&mut buffer[..count]) {
                Ok(0) => return Err("body EOF"),
                Ok(n) => {
                    upstream
                        .write_all(&buffer[..n])
                        .map_err(|_| "upstream body")?;
                    size -= n;
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                _ => return Err("body read"),
            }
        }
        Ok(())
    };
    if let Some(size) = body {
        copy_exact(size)?;
    } else {
        // Decode framing solely to ensure one bounded request. Preserve validated chunks.
        drop(copy_exact);
        let mut total = 0usize;
        loop {
            let mut line = Vec::new();
            while !line.ends_with(b"\r\n") {
                if !live() || line.len() > 128 {
                    return Err("chunk line bound");
                }
                match client.read(&mut byte) {
                    Ok(1) => line.push(byte[0]),
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        continue;
                    }
                    _ => return Err("chunk EOF"),
                }
            }
            let digits =
                std::str::from_utf8(&line[..line.len() - 2]).map_err(|_| "chunk syntax")?;
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("chunk syntax");
            }
            let size = usize::from_str_radix(digits, 16).map_err(|_| "chunk size")?;
            total = total.checked_add(size).ok_or("body bound")?;
            if total > 8 * 1024 * 1024 {
                return Err("body bound");
            }
            upstream.write_all(&line).map_err(|_| "chunk write")?;
            let mut remaining = size;
            while remaining > 0 {
                if !live() {
                    return Err("stale lease");
                }
                let count = remaining.min(buffer.len());
                match client.read(&mut buffer[..count]) {
                    Ok(0) => return Err("chunk EOF"),
                    Ok(n) => {
                        upstream
                            .write_all(&buffer[..n])
                            .map_err(|_| "chunk write")?;
                        remaining -= n;
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        continue;
                    }
                    _ => return Err("chunk read"),
                }
            }
            let mut ending = [0; 2];
            client.read_exact(&mut ending).map_err(|_| "chunk ending")?;
            if ending != *b"\r\n" {
                return Err("trailers unsupported");
            }
            upstream.write_all(&ending).map_err(|_| "chunk ending")?;
            if size == 0 {
                break;
            }
        }
    }
    // Never interpret a second request on this socket. Fixed upstream plus close
    // also prevents an absolute URI from selecting an arbitrary host destination.
    while live() {
        match upstream.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(n) => client
                .write_all(&buffer[..n])
                .map_err(|_| "response write")?,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            _ => return Err("response read"),
        }
    }
    Err("lease revoked or deadline")
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Lease(AtomicBool);
    impl LiveGatewayLease for Lease {
        fn current(&self) -> bool {
            self.0.load(Ordering::Acquire)
        }
    }
    #[test]
    fn finite_origin_and_framing() {
        let t = "127.0.0.1:1234".parse().unwrap();
        let valid=b"GET http://127.0.0.1:1234/v1/models HTTP/1.1\r\nHost: 127.0.0.1:1234\r\nAuthorization: Bearer scoped\r\n\r\n";
        assert!(request_head(valid, t).is_ok());
        for from in [
            "http://127.0.0.1:1234.evil/",
            "http://127.0.0.1:1235/",
            "http://example.com/",
            "http://127.0.0.1:1234//evil/",
        ] {
            assert!(
                request_head(
                    format!("GET {from} HTTP/1.1\r\nHost: 127.0.0.1:1234\r\n\r\n").as_bytes(),
                    t
                )
                .is_err()
            );
        }
        assert!(request_head(b"POST http://127.0.0.1:1234/ HTTP/1.1\r\nHost: 127.0.0.1:1234\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\n",t).is_err());
    }
    #[test]
    fn actual_fixed_gateway_and_revocation() {
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        let std::net::SocketAddr::V4(address) = target.local_addr().unwrap() else {
            panic!()
        };
        let lease = Arc::new(Lease(AtomicBool::new(true)));
        let bridge = GatewayBridge::start(address, lease.clone()).unwrap();
        let upstream = thread::spawn(move || {
            let (mut stream, _) = target.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut b = [0];
            while !bytes.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut b).unwrap();
                bytes.push(b[0]);
            }
            assert!(bytes.starts_with(b"GET /v1/models HTTP/1.1\r\n"));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK")
                .unwrap();
        });
        let mut client = TcpStream::connect(bridge.endpoint()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        write!(
            client,
            "GET http://{address}/v1/models HTTP/1.1\r\nHost: {address}\r\n\r\n"
        )
        .unwrap();
        let mut bytes = Vec::new();
        client.read_to_end(&mut bytes).unwrap();
        assert!(bytes.ends_with(b"OK"));
        upstream.join().unwrap();
        lease.0.store(false, Ordering::Release);
        drop(bridge);
        assert!(GatewayBridge::start(address, lease).is_err());
    }
}

#[cfg(test)]
mod hop_header_tests {
    #[test]
    fn strips_finite_proxy_connection_without_changing_authority() {
        let target = "127.0.0.1:1234".parse().unwrap();
        for value in ["keep-alive", "close", "Keep-Alive"] {
            let input = format!(
                "POST http://127.0.0.1:1234/v1/messages HTTP/1.1\r\nHost: 127.0.0.1:1234\r\nProxy-Connection: {value}\r\nAuthorization: Bearer scoped\r\nContent-Length: 2\r\n\r\n"
            );
            let (head, len) = super::request_head(input.as_bytes(), target).unwrap();
            let head = String::from_utf8(head).unwrap();
            assert!(!head.to_ascii_lowercase().contains("proxy-connection"));
            assert!(head.contains("Authorization: Bearer scoped\r\n"));
            assert!(head.contains("Connection: close\r\n"));
            assert_eq!(len, Some(2));
        }
    }
    #[test]
    fn rejects_proxy_header_duplicates_options_and_other_transport() {
        let target = "127.0.0.1:1234".parse().unwrap();
        for headers in [
            "Proxy-Connection: keep-alive, upgrade\r\n",
            "Proxy-Connection: upgrade\r\n",
            "Proxy-Connection: close\r\nProxy-Connection: close\r\n",
            "Upgrade: websocket\r\n",
            "Trailer: authorization\r\n",
            "Proxy-Authorization: secret\r\n",
        ] {
            let input = format!(
                "POST http://127.0.0.1:1234/v1/messages HTTP/1.1\r\nHost: 127.0.0.1:1234\r\n{headers}Content-Length: 0\r\n\r\n"
            );
            assert!(super::request_head(input.as_bytes(), target).is_err());
        }
        let input=b"POST http://127.0.0.1:9999/v1/messages HTTP/1.1\r\nHost: 127.0.0.1:9999\r\nProxy-Connection: keep-alive\r\n\r\n";
        assert!(super::request_head(input, target).is_err());
    }
}

#[cfg(test)]
mod lifetime_tests {
    use super::*;
    struct Lease(AtomicBool);
    impl LiveGatewayLease for Lease {
        fn current(&self) -> bool {
            self.0.load(Ordering::Acquire)
        }
    }
    #[test]
    fn dispatcher_exit_revokes_liveness_but_listener_identity_is_retained_until_drop() {
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        let std::net::SocketAddr::V4(address) = target.local_addr().unwrap() else {
            panic!()
        };
        let lease = Arc::new(Lease(AtomicBool::new(true)));
        let bridge = GatewayBridge::start(address, lease.clone()).unwrap();
        let endpoint = bridge.endpoint();
        assert!(bridge.current());
        lease.0.store(false, Ordering::Release);
        let until = std::time::Instant::now() + Duration::from_secs(2);
        while bridge.alive.load(Ordering::Acquire) {
            assert!(std::time::Instant::now() < until);
            thread::sleep(Duration::from_millis(2));
        }
        assert!(!bridge.current());
        assert!(
            TcpListener::bind(endpoint).is_err(),
            "failed dispatcher cannot permit endpoint substitution"
        );
        drop(bridge);
        let replacement = TcpListener::bind(endpoint).unwrap();
        drop(replacement);
    }
    #[test]
    fn unwinding_guard_revokes_both_status_and_worker_cancellation() {
        let alive = Arc::new(AtomicBool::new(true));
        let cancelled = Arc::new(AtomicBool::new(false));
        let a = alive.clone();
        let c = cancelled.clone();
        let joined = thread::spawn(move || {
            let _life = DispatcherLife {
                alive: a,
                cancelled: c,
            };
            panic!("owned test dispatcher failure");
        })
        .join();
        assert!(joined.is_err());
        assert!(!alive.load(Ordering::Acquire));
        assert!(cancelled.load(Ordering::Acquire));
    }
}

#[cfg(test)]
mod owned_worker_tests {
    use super::*;
    #[test]
    fn dispatcher_unwind_cancels_and_joins_its_active_workers() {
        let alive = Arc::new(AtomicBool::new(true));
        let cancel = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let a = alive.clone();
        let c = cancel.clone();
        let f = finished.clone();
        let joined = thread::spawn(move || {
            let mut owned = OwnedWorkers {
                handles: Vec::new(),
                alive: a,
                cancelled: c.clone(),
            };
            owned.push(thread::spawn(move || {
                while !c.load(Ordering::Acquire) {
                    thread::sleep(Duration::from_millis(1));
                }
                f.store(true, Ordering::Release);
            }));
            panic!("owned dispatcher QA unwind");
        })
        .join();
        assert!(joined.is_err());
        assert!(!alive.load(Ordering::Acquire));
        assert!(cancel.load(Ordering::Acquire));
        assert!(
            finished.load(Ordering::Acquire),
            "the failed dispatcher must have joined, not detached, its worker"
        );
    }
}

impl std::fmt::Debug for GatewayBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayBridge")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for OwnerBoundLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerBoundLease").finish_non_exhaustive()
    }
}
