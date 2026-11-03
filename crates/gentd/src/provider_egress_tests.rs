use std::io::Cursor;
use std::net::IpAddr;
use std::time::Duration;

use super::{ProviderApi, ReviewedProviderEgress, public_address, wire};

pub(super) fn policy(api: ProviderApi, address: std::net::SocketAddr) -> ReviewedProviderEgress {
    ReviewedProviderEgress::from_client(
        api,
        reqwest::Url::parse(&format!("http://{address}/v1/")).unwrap(),
        vec![("authorization".into(), "Bearer daemon-only-secret".into())],
        reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap(),
        &[address],
    )
    .unwrap()
}

#[test]
fn rejects_ambiguous_and_tunneling_requests_before_any_upstream() {
    let valid = "POST /v1/responses HTTP/1.1\r\nHost: 127.0.0.1:10\r\nAuthorization: Bearer test-token\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}";
    let cases = [
        valid.replace("POST /v1/responses", "CONNECT attacker.test:443"),
        valid.replace("/v1/responses", "http://attacker.test/v1/responses"),
        valid.replace("/v1/responses", "//attacker.test/v1/responses"),
        valid.replace("/v1/responses", "/v1/%72esponses"),
        valid.replace("test-token", "wrong-token"),
        valid.replace("127.0.0.1:10", "attacker.test"),
        valid.replace(
            "Content-Length: 2",
            "Content-Length: 2\r\nContent-Length: 2",
        ),
        valid.replace(
            "Content-Length: 2",
            "Content-Length: 2\r\nTransfer-Encoding: chunked",
        ),
        valid.replace("Content-Length: 2", "Content-Length: 16777217"),
        valid.replace("Content-Length: 2", "Content-Length: +2"),
        valid.replace(
            "Content-Length: 2",
            "Content-Length: 2\r\nUpgrade: websocket",
        ),
        valid.replace(
            "Content-Length: 2",
            "Content-Length: 2\r\nExpect: 100-continue",
        ),
        valid.replace("Content-Type: application/json", "Content-Type: text/plain"),
        valid.replace(
            "Content-Length: 2",
            "Content-Length: 2\r\nX-API-Key: test-token",
        ),
        valid.replace("Host:", " Host:"),
        valid.replace("\r\n", "\n"),
    ];
    for request in cases {
        assert!(
            wire::read(
                &mut Cursor::new(request.as_bytes()),
                "127.0.0.1:10",
                "test-token"
            )
            .is_err()
        );
    }
    assert!(
        wire::read(
            &mut Cursor::new(valid.as_bytes()),
            "127.0.0.1:10",
            "test-token"
        )
        .is_ok()
    );
}

#[test]
fn production_configuration_rejects_cleartext_local_and_ambient_credentials() {
    for upstream in [
        "http://api.openai.com/v1/",
        "https://127.0.0.1/v1/",
        "https://169.254.169.254/v1/",
        "https://user:secret@api.openai.com/v1/",
        "https://api.openai.com/v1/?redirect=attacker",
        "https://api.openai.com/v1/#fragment",
        "https://api.openai.com/v1",
    ] {
        assert!(ReviewedProviderEgress::new(ProviderApi::Codex, upstream, vec![]).is_err());
    }
    let url = reqwest::Url::parse("https://api.openai.com/v1/").unwrap();
    for credentials in [
        vec![],
        vec![("host".into(), "attacker.test".into())],
        vec![(
            "authorization".into(),
            "secret\r\nHost: attacker.test".into(),
        )],
    ] {
        assert!(
            ReviewedProviderEgress::from_client(
                ProviderApi::Codex,
                url.clone(),
                credentials,
                reqwest::blocking::Client::new(),
                &[]
            )
            .is_err()
        );
    }
}

#[test]
fn denies_private_and_special_addresses() {
    for address in [
        "0.0.0.0",
        "10.0.0.1",
        "100.64.0.1",
        "127.0.0.1",
        "169.254.169.254",
        "172.16.0.1",
        "192.0.0.1",
        "192.0.2.1",
        "192.168.0.1",
        "198.18.0.1",
        "198.51.100.1",
        "203.0.113.1",
        "224.0.0.1",
        "255.255.255.255",
        "::1",
        "::ffff:127.0.0.1",
    ] {
        assert!(
            !public_address(address.parse::<IpAddr>().unwrap()),
            "{address}"
        );
    }
    assert!(public_address("1.1.1.1".parse().unwrap()));
}

#[test]
fn route_and_policy_identity_bind_provider_upstream_and_pinned_address() {
    let first = policy(ProviderApi::Claude, "127.0.0.1:10".parse().unwrap());
    let changed = policy(ProviderApi::Claude, "127.0.0.1:11".parse().unwrap());
    let codex = policy(ProviderApi::Codex, "127.0.0.1:10".parse().unwrap());
    assert_ne!(first.policy_digest(), changed.policy_digest());
    assert_ne!(first.policy_digest(), codex.policy_digest());
    assert!(first.target("POST", "/v1/messages?beta=true").is_ok());
    assert!(first.target("POST", "/v1/responses").is_err());
    for path in [
        "/v1/responses?url=https://attacker.test",
        "/v1/responses/../files",
        "/v1/responses/",
        "/v1/files",
        "https://attacker.test/v1/responses",
    ] {
        assert!(codex.target("POST", path).is_err());
    }
    assert!(codex.target("GET", "/v1/models").is_ok());
    assert!(codex.target("POST", "/v1/responses/compact").is_ok());
}

#[test]
fn policy_debug_never_exposes_upstream_credentials() {
    let policy = policy(ProviderApi::Codex, "127.0.0.1:10".parse().unwrap());
    let debug = format!("{policy:?}");
    assert!(!debug.contains("daemon-only-secret"));
    assert!(!debug.contains("Bearer"));
    assert!(debug.contains(policy.policy_digest()));
}
