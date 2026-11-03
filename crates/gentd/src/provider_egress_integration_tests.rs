use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::credential_capture::CredentialCapture;
use super::tests::policy;
use super::{ProviderApi, ProviderEgressBroker};

fn delayed_request(mut stream: TcpStream, headers: &str, body: &str) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    thread::sleep(Duration::from_millis(100));
    let split = headers.len() / 2;
    stream.write_all(&headers.as_bytes()[..split]).unwrap();
    thread::sleep(Duration::from_millis(100));
    stream.write_all(&headers.as_bytes()[split..]).unwrap();
    stream.write_all(&body.as_bytes()[..1]).unwrap();
    thread::sleep(Duration::from_millis(100));
    stream.write_all(&body.as_bytes()[1..]).unwrap();
    stream.shutdown(Shutdown::Write).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

#[test]
fn delayed_partial_gateway_request_reaches_upstream() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let upstream = thread::spawn(move || {
        let mut socket = accepted(&listener);
        let mut input = BufReader::new(socket.try_clone().unwrap());
        let mut headers = String::new();
        loop {
            let mut line = String::new();
            assert!(input.read_line(&mut line).unwrap() > 0);
            if line == "\r\n" {
                break;
            }
            headers.push_str(&line);
        }
        let mut body = [0; 2];
        input.read_exact(&mut body).unwrap();
        assert_eq!(body, *b"{}");
        assert!(headers.contains("Bearer daemon-only-secret"));
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK")
            .unwrap();
    });
    let broker = ProviderEgressBroker::start(policy(ProviderApi::Codex, address)).unwrap();
    let response = delayed_request(
        TcpStream::connect(broker.endpoint()).unwrap(),
        &format!(
            "POST /v1/responses HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n",
            broker.endpoint(),
            broker.client_token()
        ),
        "{}",
    );
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("OK"), "{response}");
    assert!(response.ends_with("\r\n0\r\n\r\n"), "{response}");
    upstream.join().unwrap();
}

#[test]
fn delayed_partial_credential_capture_preserves_auth_and_stream_response() {
    let capture = CredentialCapture::start().unwrap();
    let url = reqwest::Url::parse(&capture.base_url).unwrap();
    let authority = format!("{}:{}", url.host_str().unwrap(), url.port().unwrap());
    let body = r#"{"model":"fixture-model","messages":[],"stream":true,"tools":[]}"#;
    let response = delayed_request(
        TcpStream::connect(&authority).unwrap(),
        &format!(
            "POST {}/v1/messages HTTP/1.1\r\nHost: {authority}\r\nAuthorization: Bearer fixture-capture-secret\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            url.path(),
            body.len()
        ),
        body,
    );
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("event: message_stop\n"));
    assert!(!response.contains("fixture-capture-secret"));
    let auth = capture.finish().unwrap();
    assert!(auth.headers.contains(&(
        "authorization".into(),
        "Bearer fixture-capture-secret".into()
    )));
    assert!(!format!("{auth:?}").contains("fixture-capture-secret"));
}

#[test]
fn revocation_closes_an_incomplete_gateway_request_without_upstream() {
    let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
    upstream.set_nonblocking(true).unwrap();
    let broker =
        ProviderEgressBroker::start(policy(ProviderApi::Codex, upstream.local_addr().unwrap()))
            .unwrap();
    let mut stream = TcpStream::connect(broker.endpoint()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
        .write_all(b"POST /v1/responses HTTP/1.1\r\n")
        .unwrap();
    thread::sleep(Duration::from_millis(100));
    broker.revoke();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.is_empty(), "{response}");
    assert!(upstream.accept().is_err());
}

fn accepted(listener: &TcpListener) -> TcpStream {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                return stream;
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("upstream fixture accept failed: {error}"),
        }
    }
}

fn raw(broker: &ProviderEgressBroker, request: &str) -> String {
    let mut stream = TcpStream::connect(broker.endpoint()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    stream.shutdown(Shutdown::Write).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

#[test]
fn real_gateway_streams_and_replaces_credentials_without_forwarding_host_or_cookie() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let upstream = thread::spawn(move || {
        let mut socket = accepted(&listener);
        let mut input = BufReader::new(socket.try_clone().unwrap());
        let mut headers = String::new();
        loop {
            let mut line = String::new();
            assert!(input.read_line(&mut line).unwrap() > 0);
            if line == "\r\n" {
                break;
            }
            headers.push_str(&line);
        }
        let mut body = [0; 2];
        input.read_exact(&mut body).unwrap();
        tx.send((headers, body)).unwrap();
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: first\n\n").unwrap();
        socket.flush().unwrap();
        release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        socket.write_all(b"data: second\n\n").unwrap();
    });
    let broker = ProviderEgressBroker::start(policy(ProviderApi::Codex, address)).unwrap();
    let mut response = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
        .post(format!("{}/responses", broker.client_base_url()))
        .bearer_auth(broker.client_token())
        .header("content-type", "application/json")
        .header("cookie", "client-secret")
        .body("{}")
        .send()
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let mut first = vec![0; b"data: first\n\n".len()];
    response.read_exact(&mut first).unwrap();
    assert_eq!(first, b"data: first\n\n");
    release_tx.send(()).unwrap();
    let mut second = String::new();
    response.read_to_string(&mut second).unwrap();
    assert_eq!(second, "data: second\n\n");
    let (headers, body) = rx.recv_timeout(Duration::from_secs(3)).unwrap();
    let headers = headers.to_ascii_lowercase();
    assert!(headers.starts_with("post /v1/responses http/1.1\r\n"));
    assert!(headers.contains("authorization: bearer daemon-only-secret\r\n"));
    assert!(!headers.contains(broker.client_token()));
    assert!(!headers.contains("cookie:"));
    assert_eq!(body, *b"{}");
    upstream.join().unwrap();
    let endpoint = broker.endpoint();
    drop(broker);
    assert!(TcpStream::connect_timeout(&endpoint, Duration::from_millis(200)).is_err());
}

#[test]
fn real_listener_rejects_tunnels_foreign_paths_and_bad_tokens_without_connecting_upstream() {
    let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
    upstream.set_nonblocking(true).unwrap();
    let broker =
        ProviderEgressBroker::start(policy(ProviderApi::Codex, upstream.local_addr().unwrap()))
            .unwrap();
    for (method, path, token, status) in [
        ("CONNECT", "attacker.test:443", broker.client_token(), "400"),
        ("GET", "/v1/files", broker.client_token(), "400"),
        ("GET", "/v1/models", "stolen-wrong-token", "401"),
    ] {
        let result = raw(
            &broker,
            &format!(
                "{method} {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {token}\r\n\r\n",
                broker.endpoint()
            ),
        );
        assert!(
            result.starts_with(&format!("HTTP/1.1 {status}")),
            "{result}"
        );
    }
    assert!(upstream.accept().is_err());
}

#[test]
fn real_upstream_redirect_is_neither_followed_nor_exposed() {
    let sink = TcpListener::bind("127.0.0.1:0").unwrap();
    sink.set_nonblocking(true).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let redirect = format!(
        "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{}/secret\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        sink.local_addr().unwrap()
    );
    let upstream = thread::spawn(move || {
        let mut socket = accepted(&listener);
        let mut input = BufReader::new(socket.try_clone().unwrap());
        loop {
            let mut line = String::new();
            assert!(input.read_line(&mut line).unwrap() > 0);
            if line == "\r\n" {
                break;
            }
        }
        socket.write_all(redirect.as_bytes()).unwrap();
    });
    let broker = ProviderEgressBroker::start(policy(ProviderApi::Codex, address)).unwrap();
    let result = raw(
        &broker,
        &format!(
            "GET /v1/models HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\n\r\n",
            broker.endpoint(),
            broker.client_token()
        ),
    );
    assert!(result.starts_with("HTTP/1.1 502"));
    assert!(!result.contains("Location:"));
    assert!(!result.contains("secret"));
    assert!(sink.accept().is_err());
    upstream.join().unwrap();
}
