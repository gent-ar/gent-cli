use std::path::PathBuf;
use std::sync::Mutex;

use gent_types::{RunVersionLock, SandboxLaunchProfile, SandboxResourceLimits};
use serde_json::Value;

use super::*;

#[derive(Clone, Copy, Debug)]
struct TransportError;

struct Transport(Mutex<Vec<Vec<u8>>>, Mutex<Result<Vec<u8>, TransportError>>);

impl Default for Transport {
    fn default() -> Self {
        Self(Mutex::new(vec![]), Mutex::new(Err(TransportError)))
    }
}

impl MacosProviderHelperTransport for Transport {
    type Error = TransportError;

    fn exchange(&self, request: &[u8]) -> Result<Vec<u8>, Self::Error> {
        self.0.lock().unwrap().push(request.to_vec());
        self.1.lock().unwrap().clone()
    }
}

fn request() -> SandboxedLaunchRequest {
    SandboxedLaunchRequest {
        lock: RunVersionLock {
            provider: "codex".into(),
            canonical_path: "/private/codex".into(),
            file_identity: "1:2".into(),
            digest_sha256: "a".repeat(64),
            version: "1".into(),
            compatibility_entry: "codex-1".into(),
        },
        profile: SandboxLaunchProfile::new(
            std::path::Path::new("/workspace"),
            &[PathBuf::from("/workspace")],
            &[],
            vec![],
            SandboxNetworkPolicy::Disabled,
            SandboxResourceLimits {
                max_processes: 1,
                max_memory_bytes: 1,
                max_cpu_time_ms: 1,
            },
        )
        .unwrap(),
    }
}

fn response(request_id: &str, reason: &str) -> Vec<u8> {
    format!(
        r#"{{"protocolVersion":1,"requestId":"{request_id}","helper":{{"bundleId":"{BUNDLE_ID}","version":"{HELPER_VERSION}"}},"result":{{"state":"denied","reason":"{reason}"}}}}"#
    )
    .into_bytes()
}

#[test]
fn protocol_client_sends_no_launch_data_and_preserves_the_profile_digest() {
    let transport = Transport(
        Mutex::new(vec![]),
        Mutex::new(Ok(response("one", "containmentSemanticsUnavailable"))),
    );
    let client = MacosProviderHelperClient::new(transport);
    assert_eq!(
        client
            .prepare(
                &request(),
                &MacosHelperPrepare {
                    request_id: "one".into(),
                    workspace_bookmark: None,
                },
            )
            .unwrap(),
        MacosHelperDenial::ContainmentSemanticsUnavailable
    );
    let payload = client.transport.0.lock().unwrap();
    let json: Value = serde_json::from_slice(&payload[0]).unwrap();
    assert_eq!(json["operation"], "prepare");
    assert!(json.get("launch").is_none());
    assert!(json.get("arguments").is_none());
    assert_eq!(
        json["profile"]["profileDigestSha256"],
        request().profile.digest_sha256()
    );
}

#[test]
fn altered_response_never_authorizes_a_launch() {
    let transport = Transport(
        Mutex::new(vec![]),
        Mutex::new(Ok(response("other", "containmentSemanticsUnavailable"))),
    );
    let client = MacosProviderHelperClient::new(transport);
    assert_eq!(
        client.prepare(
            &request(),
            &MacosHelperPrepare {
                request_id: "one".into(),
                workspace_bookmark: None,
            },
        ),
        Err(MacosProviderHelperError::RequestMismatch)
    );
}

#[test]
fn malformed_bookmark_never_reaches_a_transport() {
    let transport = Transport::default();
    let client = MacosProviderHelperClient::new(transport);
    assert_eq!(
        client.prepare(
            &request(),
            &MacosHelperPrepare {
                request_id: "one".into(),
                workspace_bookmark: Some("not-base64".into()),
            },
        ),
        Err(MacosProviderHelperError::InvalidWorkspaceBookmark)
    );
    assert!(client.transport.0.lock().unwrap().is_empty());
}
