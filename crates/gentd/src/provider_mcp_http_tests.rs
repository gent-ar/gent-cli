use crate::provider_egress::{
    ProviderApi, ProviderEgressBroker,
    mcp::{self, Admission, Authorizer, Call, Decision, Journal, Peer, Route},
    tests::policy,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
struct Policy;
impl Authorizer for Policy {
    fn decide(&self, _: &Call<'_>) -> Decision {
        Decision::Deny
    }
}
#[derive(Default)]
struct Memory(Mutex<BTreeMap<String, Recorded>>);
type Recorded = (Vec<u8>, Option<Value>);
impl Journal for Memory {
    fn reserve(&self, k: &str, b: &[u8]) -> Result<Admission, mcp::Error> {
        let mut m = self.0.lock().unwrap();
        if let Some((old, v)) = m.get(k) {
            if old != b {
                return Err(mcp::Error::Protocol);
            }
            return Ok(v.clone().map_or(Admission::Unknown, Admission::Cached));
        }
        m.insert(k.into(), (b.into(), None));
        Ok(Admission::Fresh)
    }
    fn complete(&self, k: &str, v: &Value) -> Result<(), mcp::Error> {
        self.0.lock().unwrap().get_mut(k).unwrap().1 = Some(v.clone());
        Ok(())
    }
    fn unknown(&self, _: &str) {}
}
struct TestPeer(Arc<AtomicUsize>);
impl Peer for TestPeer {
    fn request(&self, v: &Value) -> Result<Value, mcp::Error> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"jsonrpc":"2.0","id":v["id"],"result":{"tools":[]}}))
    }
    fn notify(&self, _: &Value) -> Result<(), mcp::Error> {
        Ok(())
    }
    fn revoke(&self) {}
    fn settled(&self) -> bool {
        true
    }
}
fn fixture() -> (ProviderEgressBroker, Arc<AtomicUsize>, TcpListener) {
    let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
    upstream.set_nonblocking(true).unwrap();
    let broker =
        ProviderEgressBroker::start(policy(ProviderApi::Claude, upstream.local_addr().unwrap()))
            .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut routes = BTreeMap::new();
    for i in 0..7 {
        routes.insert(
            format!("{i:032x}"),
            Route::new(
                format!("gent-server-{i}"),
                Arc::new(TestPeer(calls.clone())),
            )
            .unwrap(),
        );
    }
    let bridge = mcp::Bridge::new(
        broker.client_token().into(),
        routes,
        Arc::new(|| true),
        Arc::new(Policy),
        Arc::new(Memory::default()),
    )
    .unwrap();
    broker.attach_mcp(Arc::new(bridge)).unwrap();
    (broker, calls, upstream)
}
fn request(
    b: &ProviderEgressBroker,
    method: &str,
    path: &str,
    token: &str,
    extra: &str,
    body: &str,
) -> String {
    let mut s = TcpStream::connect(b.endpoint()).unwrap();
    s.set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .unwrap();
    write!(s,"{method} {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}\r\n{body}",b.endpoint(),body.len()).unwrap();
    let mut result = String::new();
    s.read_to_string(&mut result).unwrap();
    result
}
#[test]
fn host_mcp_http_seven_routes_catalogue_without_any_model_upstream() {
    let (b, calls, upstream) = fixture();
    for i in 0..7 {
        let response = request(
            &b,
            "POST",
            &format!("/__gent_mcp/{i:032x}"),
            b.client_token(),
            "",
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        );
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.ends_with(r#"{"id":1,"jsonrpc":"2.0","result":{"tools":[]}}"#));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 7);
    assert!(upstream.accept().is_err());
    b.revoke();
    assert!(b.mcp_settled());
}
#[test]
fn host_mcp_http_auth_origin_route_and_tool_denial_precede_effect() {
    let (b, calls, upstream) = fixture();
    let path = format!("/__gent_mcp/{:032x}", 0);
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"write"}}"#;
    assert!(request(&b, "POST", &path, "wrong", "", body).starts_with("HTTP/1.1 401"));
    assert!(
        request(
            &b,
            "POST",
            &path,
            b.client_token(),
            "Origin: http://evil.invalid\r\n",
            body
        )
        .starts_with("HTTP/1.1 401")
    );
    assert!(
        request(
            &b,
            "POST",
            &format!("/__gent_mcp/{}", "f".repeat(32)),
            b.client_token(),
            "",
            body
        )
        .starts_with("HTTP/1.1 404")
    );
    assert!(request(&b, "POST", &path, b.client_token(), "", body).starts_with("HTTP/1.1 403"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(upstream.accept().is_err());
}
#[test]
fn host_mcp_http_notifications_and_idle_get_are_bounded() {
    let (b, calls, _) = fixture();
    let path = format!("/__gent_mcp/{:032x}", 0);
    assert!(
        request(
            &b,
            "POST",
            &path,
            b.client_token(),
            "",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
        )
        .starts_with("HTTP/1.1 202")
    );
    assert!(request(&b, "GET", &path, b.client_token(), "", "").starts_with("HTTP/1.1 405"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
