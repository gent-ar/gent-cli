use super::*;

#[test]
fn actual_capture_accepts_one_hello_then_authenticated_message() {
    let capture = CredentialCapture::start().unwrap();
    let (authority, nonce) = capture
        .base_url
        .strip_prefix("http://")
        .unwrap()
        .split_once('/')
        .unwrap();
    let roundtrip = |request: String| {
        let mut socket = TcpStream::connect(authority).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        socket.write_all(request.as_bytes()).unwrap();
        let mut bytes = Vec::new();
        socket.read_to_end(&mut bytes).unwrap();
        bytes
    };
    let hello = roundtrip(format!(
        "HEAD /{nonce}/api/hello HTTP/1.1\r\nHost: {authority}\r\n\r\n"
    ));
    assert!(hello.starts_with(b"HTTP/1.1 200 OK\r\n"));
    let body = r#"{"model":"synthetic","stream":true,"tools":[]}"#;
    let response = roundtrip(format!(
        "POST /{nonce}/v1/messages HTTP/1.1\r\nHost: {authority}\r\nAuthorization: Bearer synthetic-test\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    ));
    assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));
    let auth = capture.finish().unwrap();
    assert_eq!(
        auth.headers,
        vec![(
            "authorization".to_string(),
            "Bearer synthetic-test".to_string()
        )]
    );
}

type Captured = Result<Option<(CapturedProviderAuth, String)>, EgressError>;
fn exchange(request: String, allow_hello: bool) -> (Captured, Vec<u8>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let authority = address.to_string();
    let request = request.replace("AUTHORITY", &authority);
    let worker = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        read(&mut socket, &authority, "nonce", allow_hello)
    });
    let mut client = TcpStream::connect(address).unwrap();
    client.write_all(request.as_bytes()).unwrap();
    let mut output = Vec::new();
    client.read_to_end(&mut output).unwrap();
    (worker.join().unwrap(), output)
}

#[test]
fn bounded_nonce_hello_has_no_credential_authority() {
    let (result, response) = exchange(
        "HEAD /nonce/api/hello HTTP/1.1\r\nHost: AUTHORITY\r\n\r\n".into(),
        true,
    );
    assert!(matches!(result, Ok(None)));
    assert_eq!(
        response,
        b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
}

#[test]
fn malformed_or_repeated_hello_fails_closed() {
    for (path, host, header, allowed) in [
        ("/nonce/api/hello", "AUTHORITY", "", false),
        ("/wrong/api/hello", "AUTHORITY", "", true),
        ("/nonce/api/hello", "wrong", "", true),
        (
            "/nonce/api/hello",
            "AUTHORITY",
            "Authorization: Bearer synthetic\r\n",
            true,
        ),
        (
            "/nonce/api/hello",
            "AUTHORITY",
            "X-Api-Key: synthetic\r\n",
            true,
        ),
        (
            "/nonce/api/hello",
            "AUTHORITY",
            "Content-Length: 1\r\n",
            true,
        ),
        (
            "/nonce/api/hello",
            "AUTHORITY",
            "Transfer-Encoding: chunked\r\n",
            true,
        ),
        ("/nonce/api/hello", "AUTHORITY", "Host: AUTHORITY\r\n", true),
    ] {
        let (result, response) = exchange(
            format!("HEAD {path} HTTP/1.1\r\nHost: {host}\r\n{header}\r\n"),
            allowed,
        );
        assert!(result.is_err());
        assert!(response.is_empty());
    }
}

#[test]
fn post_still_requires_authentication_and_streaming_tool_free_body() {
    let body = r#"{"model":"synthetic","stream":true,"tools":[]}"#;
    let request = format!(
        "POST /nonce/v1/messages?beta=true HTTP/1.1\r\nHost: AUTHORITY\r\nAuthorization: Bearer synthetic-test\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let (result, _) = exchange(request.clone(), false);
    assert!(matches!(result, Ok(Some(_))));
    let (result, _) = exchange(
        request.replace("Authorization: Bearer synthetic-test\r\n", ""),
        false,
    );
    assert!(result.is_err());
}
