use super::*;
use std::{
    cell::Cell,
    io::{Read, Write},
    net::TcpStream,
};

fn fixture_resolve(
    lock: &RunVersionLock,
    parent: &std::path::Path,
    environment: BTreeMap<String, String>,
    run: impl Fn(
        &RunVersionLock,
        &std::path::Path,
        &[String],
        &BTreeMap<String, String>,
        Duration,
    ) -> Result<probe_process::ProbeOutput, probe_process::ProbeFailure>,
) -> Result<CapturedProviderAuth, EgressError> {
    super::resolve_with_owner(
        lock,
        parent,
        environment,
        run,
        &probe_owner::Owner::default(),
    )
}

fn lock() -> RunVersionLock {
    RunVersionLock {
        provider: "claude".into(),
        canonical_path: "unused-fake-runner".into(),
        file_identity: "fixture".into(),
        digest_sha256: "a".repeat(64),
        version: "fixture".into(),
        compatibility_entry: "fixture".into(),
    }
}

#[test]
#[cfg_attr(windows, ignore = "requires exact pinned native cleanup cohort")]
fn actual_resolver_success_removes_only_settled_probe_scaffolding() {
    let parent = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let calls = Cell::new(0);
    let result=fixture_resolve(&lock(),parent.path(),BTreeMap::new(),|_,cwd,_,env,_| {
        calls.set(calls.get()+1);
        if calls.get()==1 {
            return Ok(probe_process::ProbeOutput {success:true,stderr_empty:true,stdout:json!({"authMethod":"claude.ai","loggedIn":true,"configDirectory":config.path()}).to_string().into_bytes()});
        }
        // Only this public synthetic bearer is used; no installed SDK, auth
        // file, subscription request or global environment is accessed.
        std::fs::create_dir(cwd.parent().unwrap().join("claude")).unwrap();
        let base=env["ANTHROPIC_BASE_URL"].strip_prefix("http://").unwrap();let (authority,nonce)=base.split_once('/').unwrap();
        let body=r#"{"model":"synthetic","stream":true,"tools":[]}"#;
        let mut socket=TcpStream::connect(authority).unwrap();socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        write!(socket,"POST /{nonce}/v1/messages HTTP/1.1\r\nHost: {authority}\r\nAuthorization: Bearer synthetic-test\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",body.len()).unwrap();
        let mut response=Vec::new();socket.read_to_end(&mut response).unwrap();assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));
        let stdout=b"{\"type\":\"system\",\"subtype\":\"init\",\"tools\":[],\"mcp_servers\":[],\"plugins\":[],\"slash_commands\":[]}\n{\"type\":\"result\",\"result\":\"OK\",\"is_error\":false}\n".to_vec();
        Ok(probe_process::ProbeOutput {success:true,stderr_empty:true,stdout})
    }).unwrap();
    assert_eq!(calls.get(), 2);
    assert_eq!(result.headers.len(), 1);
    assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(config.path()).unwrap().count(), 0);
}

#[test]
#[cfg_attr(windows, ignore = "requires exact pinned native cleanup cohort")]
fn actual_resolver_known_nonzero_and_invalid_status_failures_clean() {
    for success in [false, true] {
        let parent = tempfile::tempdir().unwrap();
        assert!(
            fixture_resolve(&lock(), parent.path(), BTreeMap::new(), |_, _, _, _, _| Ok(
                probe_process::ProbeOutput {
                    success,
                    stderr_empty: true,
                    stdout: b"invalid-status".to_vec()
                }
            ))
            .is_err()
        );
        assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
    }
}

#[test]
#[cfg_attr(windows, ignore = "requires exact pinned native cleanup cohort")]
fn actual_resolver_unknown_retains_one_owner_and_rejects_repeated_capture() {
    let parent = tempfile::tempdir().unwrap();
    let owner = probe_owner::Owner::default();
    let calls = Cell::new(0);
    let run = |_: &RunVersionLock,
               _: &std::path::Path,
               _: &[String],
               _: &BTreeMap<String, String>,
               _: Duration| {
        calls.set(calls.get() + 1);
        Err(probe_process::ProbeFailure::unknown())
    };
    assert!(
        super::resolve_with_owner(&lock(), parent.path(), BTreeMap::new(), run, &owner).is_err()
    );
    assert_eq!(calls.get(), 1);
    assert!(
        super::resolve_with_owner(&lock(), parent.path(), BTreeMap::new(), run, &owner).is_err()
    );
    assert_eq!(calls.get(), 1);
    let roots = std::fs::read_dir(parent.path())
        .unwrap()
        .map(|v| v.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 1);
    assert!(roots[0].join("cwd").is_dir());
    // This injected Unknown never spawned a child. Owner teardown releases
    // its tracked test capability before TempDir removes its own empty tree.
}

#[test]
#[cfg_attr(windows, ignore = "requires exact pinned native cleanup cohort")]
fn authenticated_known_read_or_size_failure_cleans_instead_of_latching() {
    let parent = tempfile::tempdir().unwrap();
    let owner = probe_owner::Owner::default();
    let calls = Cell::new(0);
    for _ in 0..2 {
        assert!(
            super::resolve_with_owner(
                &lock(),
                parent.path(),
                BTreeMap::new(),
                |_, _, _, _, _| {
                    calls.set(calls.get() + 1);
                    Err(probe_process::ProbeFailure::known())
                },
                &owner
            )
            .is_err()
        );
        assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
    }
    assert_eq!(calls.get(), 2);
}

#[test]
#[cfg_attr(windows, ignore = "requires exact pinned native cleanup cohort")]
fn unexpected_artifact_cleanup_failure_is_bounded_by_same_owner_slot() {
    let parent = tempfile::tempdir().unwrap();
    let owner = probe_owner::Owner::default();
    let calls = Cell::new(0);
    let run = |_: &RunVersionLock,
               cwd: &std::path::Path,
               _: &[String],
               _: &BTreeMap<String, String>,
               _: Duration| {
        calls.set(calls.get() + 1);
        std::fs::File::create(cwd.join("oversized"))
            .unwrap()
            .set_len(2 * 1024 * 1024 + 1)
            .unwrap();
        Ok(probe_process::ProbeOutput {
            success: true,
            stderr_empty: true,
            stdout: br#"{"authMethod":"claude.ai","loggedIn":false}"#.to_vec(),
        })
    };
    for _ in 0..2 {
        let failure =
            super::resolve_with_owner(&lock(), parent.path(), BTreeMap::new(), run, &owner)
                .unwrap_err();
        assert!(matches!(
            failure,
            EgressError::CredentialProbe {
                phase: CredentialProbePhase::LoginUnavailable,
                cleanup_failed: true,
                cleanup_unproven: false
            }
        ));
    }
    assert_eq!(calls.get(), 1);
    assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 1);
}
