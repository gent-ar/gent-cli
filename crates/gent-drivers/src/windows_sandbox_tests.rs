use super::*;

#[test]
fn helper_handshake_binds_generation_profile_and_run() {
    let digest = "a".repeat(64);
    let line = format!("GENT-SANDBOX/1 123:456 {digest} run-1\n");
    assert_eq!(
        generation(line.as_bytes(), &digest, "run-1"),
        Some("123:456".into())
    );
    assert!(generation(line.as_bytes(), &digest, "run-2").is_none());
    assert!(generation(line.as_bytes(), &"b".repeat(64), "run-1").is_none());
    assert!(generation(line.trim_end().as_bytes(), &digest, "run-1").is_none());
    assert!(
        generation(
            format!("GENT-SANDBOX/1 0:456 {digest} run-1\n").as_bytes(),
            &digest,
            "run-1"
        )
        .is_none()
    );
}

#[test]
fn gateway_handshake_requires_exact_native_peer_identity_and_generation() {
    let digest = "a".repeat(64);
    let base = format!("GENT-SANDBOX/1 123:456 {digest} run-1");
    let identity = "Gent.QA.Proxy.20261029_bh4pzfjvbfcs0";
    let package = "Gent.QA.Proxy.20261029_1.0.0.0_x64__bh4pzfjvbfcs0";
    let nonce = "b".repeat(32);
    let line = format!("{base} package={identity},{package},{nonce},54321,321,654\n");
    assert!(handshake(line.as_bytes(), &digest, "run-1", false).is_none());
    let (_, peer) = handshake(line.as_bytes(), &digest, "run-1", true).unwrap();
    assert_eq!(peer.unwrap().identity(), identity);
    assert!(handshake(format!("{base}\n").as_bytes(), &digest, "run-1", true).is_none());
    assert!(
        handshake(
            format!("{base} package={identity},{package},{nonce},0,321,654\n").as_bytes(),
            &digest,
            "run-1",
            true
        )
        .is_none()
    );
}

#[test]
fn active_gateway_rejects_legacy_or_provider_self_attestation() {
    let digest = "a".repeat(64);
    let base = format!("GENT-SANDBOX/1 123:456 {digest} run-1");
    let nonce = "b".repeat(32);
    assert!(
        handshake(
            format!("{base} peer=GentGateway-{nonce},65025,321,654\n").as_bytes(),
            &digest,
            "run-1",
            true
        )
        .is_none()
    );
    assert!(handshake(format!("{base} package=Gent.QA.Proxy.20261029_bh4pzfjvbfcs0,Gent.QA.Proxy.20261029_1.0.0.0_x64__bh4pzfjvbfcs0,{nonce},65025,123,456\n").as_bytes(), &digest, "run-1", true).is_none());
}

#[test]
fn package_handshake_preserves_exact_run_nonce_for_admission() {
    let digest = "a".repeat(64);
    let nonce = "b".repeat(32);
    let line = format!(
        "GENT-SANDBOX/1 123:456 {digest} run-1 package=Gent.QA.Proxy.20261029_bh4pzfjvbfcs0,Gent.QA.Proxy.20261029_1.0.0.0_x64__bh4pzfjvbfcs0,{nonce},65025,321,654\n"
    );
    let (_, peer) = handshake(line.as_bytes(), &digest, "run-1", true).unwrap();
    let peer = peer.unwrap();
    assert!(peer.matches_active_gateway(&nonce));
    assert!(!peer.matches_active_gateway(&"c".repeat(32)));
    assert!(handshake(line.as_bytes(), &digest, "other-run", true).is_none());
}

#[test]
fn launch_without_a_brokered_command_environment_is_rejected_before_any_spawn() {
    let lock = RunVersionLock {
        provider: "claude".into(),
        canonical_path: "C:/qa/does-not-exist/claude.exe".into(),
        file_identity: "1:2".into(),
        digest_sha256: "b".repeat(64),
        version: "1".into(),
        compatibility_entry: "fixture".into(),
    };
    let gateway = gent_types::SandboxProviderGateway::new(
        gent_types::SandboxGatewayApi::Claude,
        "127.0.0.1:54321".parse().unwrap(),
        "c".repeat(64),
        "a".repeat(64),
    )
    .unwrap();
    let workspace = gent_testkit::host_absolute_path("/workspace");
    let profile = gent_types::SandboxLaunchPolicy::new(
        vec![],
        SandboxNetworkPolicy::ReviewedEgress {
            policy_digest_sha256: "a".repeat(64),
        },
        gent_types::SandboxResourceLimits {
            max_processes: 1,
            max_memory_bytes: 1024,
            max_cpu_time_ms: 1000,
        },
    )
    .unwrap()
    .profile_for_workspace(&workspace, gent_types::SandboxWorkspaceAccess::ReadOnly)
    .unwrap()
    .with_gateway(gateway)
    .unwrap();
    let launch = ProviderLaunch {
        permission_binding: None,
        run_id: Some("run-1".into()),
        lock: lock.clone(),
        provider: "claude".into(),
        executable: lock.canonical_path.clone().into(),
        arguments: vec![],
        intent: crate::LaunchIntent::Start,
        workspace_root: Some(workspace),
        workspace_access: gent_types::SandboxWorkspaceAccess::ReadOnly,
    };
    let request = SandboxedLaunchRequest {
        lock: lock.clone(),
        profile,
    };
    let result = WindowsSandboxLauncher::new(lock, 1024).launch_sandboxed(&request, &launch);
    assert!(matches!(result, Err(Error::ProfileRejected)));
}
