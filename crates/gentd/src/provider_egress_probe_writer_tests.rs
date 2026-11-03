use super::*;
use crate::provider_egress::probe_process::environment;
use std::path::Path;
#[test]
#[ignore = "explicit native probe helper image in GENT_NATIVE_PROBE_TEST_IMAGE"]
fn exact_parent_control_writer_survives_wait_and_closes_after_known_settlement() {
    let image = std::env::var_os("GENT_NATIVE_PROBE_TEST_IMAGE").unwrap();
    let helper = gent_drivers::lock::capture(
        "windows-sandbox-helper",
        Path::new(&image),
        "1",
        "writer-test",
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
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let output = run_observed(
        helper,
        program,
        root.path().to_path_buf(),
        vec![
            "/d".into(),
            "/c".into(),
            "echo WRITER_HELD_THROUGH_WAIT".into(),
        ],
        environment(),
        Duration::from_secs(5),
        Observer {
            events: Some(events.clone()),
        },
    )
    .ok()
    .unwrap();
    assert!(output.success);
    assert_eq!(output.stdout, b"WRITER_HELD_THROUGH_WAIT\r\n");
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            ("control-held", false),
            ("scope-known", true),
            ("control-closed", true)
        ]
    );
}
