use super::*;
fn frame(flags: u32, code: u32, data: &[u8]) -> Vec<u8> {
    let mut bytes = b"GENTPRB2".to_vec();
    for word in [flags, code, u32::try_from(data.len()).unwrap()] {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes.extend_from_slice(data);
    bytes
}
#[test]
fn exact_native_frame_distinguishes_known_success_nonzero_and_read_failure() {
    let success = decode(&frame(7, 0, b"public")).unwrap().ok().unwrap();
    assert!(success.success && success.stderr_empty);
    assert_eq!(success.stdout, b"public");
    let nonzero = decode(&frame(6, 7, b"")).unwrap().ok().unwrap();
    assert!(!nonzero.success);
    let failure = decode(&frame(3, 0, b"")).unwrap().err().unwrap();
    assert!(failure.known);
    assert!(decode(&frame(0, 126, b"")).unwrap().err().unwrap().known);
}
#[test]
fn missing_malformed_oversized_or_conflicting_frame_is_unknown() {
    assert!(decode(b"").is_none());
    assert!(decode(&frame(15, 0, b"")).is_none());
    assert!(decode(&frame(7, 7, b"")).is_none());
    assert!(decode(&frame(7, 0, &vec![b'a'; 65537])).is_none());
    let mut trailing = frame(7, 0, b"public");
    trailing.push(0);
    assert!(decode(&trailing).is_none());
}
#[test]
fn bounded_worker_deadline_retains_the_actual_unfinished_owner() {
    let (release, wait) = std::sync::mpsc::channel();
    let worker = thread::spawn(move || {
        wait.recv().unwrap();
        Ok(ProbeOutput {
            success: true,
            stderr_empty: true,
            stdout: vec![],
        })
    });
    let started = Instant::now();
    let failure = await_worker(worker, started + Duration::from_millis(20))
        .err()
        .unwrap();
    assert!(!failure.known && failure.retained.is_some());
    assert!(started.elapsed() < Duration::from_secs(1));
    release.send(()).unwrap();
    drop(failure);
}
#[cfg(windows)]
#[test]
#[ignore = "explicit native probe helper image in GENT_NATIVE_PROBE_TEST_IMAGE"]
fn actual_native_transport_delivers_known_success_and_oversize_failure() {
    let helper = std::env::var_os("GENT_NATIVE_PROBE_TEST_IMAGE")
        .expect("exact guard-pinned native fixture image");
    let helper = gent_drivers::lock::capture(
        "windows-sandbox-helper",
        Path::new(&helper),
        "1",
        "native-probe-test",
    )
    .unwrap();
    let program = gent_drivers::lock::capture(
        "probe-test",
        Path::new("C:/Windows/System32/cmd.exe"),
        "windows",
        "test",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let output = run_with_helper(
        helper.clone(),
        program.clone(),
        root.path().to_path_buf(),
        vec!["/d".into(), "/c".into(), "echo GENT_RUST_NATIVE".into()],
        environment(),
        Duration::from_secs(5),
    )
    .ok()
    .unwrap();
    assert!(output.success);
    assert_eq!(output.stdout, b"GENT_RUST_NATIVE\r\n");
    let error = run_with_helper(
        helper,
        program,
        root.path().to_path_buf(),
        vec![
            "/d".into(),
            "/c".into(),
            "for /l %i in (1,1,20000) do @echo public".into(),
        ],
        environment(),
        Duration::from_secs(5),
    )
    .err()
    .unwrap();
    assert!(error.known);
}

#[test]
fn worker_panic_keeps_outer_image_authority_in_typed_unknown() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let authority = std::sync::Arc::new(vec![file.reopen().unwrap()]);
    let weak = std::sync::Arc::downgrade(&authority);
    let worker =
        thread::spawn(|| -> Result<ProbeOutput, ProbeFailure> { panic!("synthetic worker panic") });
    let failure = await_with_authority(worker, Instant::now() + Duration::from_secs(2), authority)
        .err()
        .unwrap();
    assert!(!failure.known && weak.upgrade().is_some());
    drop(failure);
    assert!(weak.upgrade().is_none());
}
